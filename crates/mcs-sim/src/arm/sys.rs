//! Machine-wide services shared by the memory-mapped peripherals of an ARM microcontroller: the
//! GPIO electrical model (pins, alternate-function routing, logic-analyzer trace), the clock tree
//! seen by the peripherals, RCC clock gating state, warnings and the Serial Monitor output.
//!
//! The CPU cycle counter counts HCLK cycles; [`ArmSys::clock`] converts cycles to seconds across
//! clock-tree changes. Peripherals schedule their events in cycles (HCLK) and derive their own tick
//! lengths from [`ClockTree`] (APB prescalers are integers, so every ratio is exact).

use std::collections::HashSet;

use crate::avr::Message;
use crate::pins::{ClockModel, Pin, PinTrace};

/// Number of distinct routed signals (peripheral number << 4 | function).
pub const NSIG: usize = 256;
/// `route` entry meaning "no pin".
pub const NO_PIN: u16 = u16::MAX;

/// Signal ids of the routed alternate functions. `peripheral << 4 | function`; id 0 is "none".
pub mod sig {
    /// UART/USART instances: 1-5 USART1..UART5, 6 LPUART1.
    pub const fn uart(p: u8, rx: bool) -> u16 {
        ((p as u16) << 4) | rx as u16
    }
    /// General-purpose timer channel: `timer` 2..4 (TIMx), `ch` 0..3.
    pub const fn tim(timer: u8, ch: u8) -> u16 {
        (((timer as u16) + 6) << 4) | (ch as u16 + 2)
    }
    pub const NONE: u16 = 0;

    /// Input signals (UART RX): the pin stays an input, the peripheral samples it.
    pub const fn is_input(s: u16) -> bool {
        s >> 4 >= 1 && s >> 4 <= 6 && s & 0xf == 1
    }
}

/// Maps a signal name from the device description (`USART1_TX`, `LPUART1_RX`, `TIM3_CH2`) to an id.
pub fn signal_id(name: &str) -> u16 {
    let (periph, f) = match name.split_once('_') {
        Some(x) => x,
        None => return sig::NONE,
    };
    let uart = |p: u8| match f {
        "TX" => sig::uart(p, false),
        "RX" => sig::uart(p, true),
        _ => sig::NONE,
    };
    match periph {
        "USART1" => uart(1),
        "USART2" => uart(2),
        "USART3" => uart(3),
        "UART4" => uart(4),
        "UART5" => uart(5),
        "LPUART1" => uart(6),
        "TIM2" | "TIM3" | "TIM4" => match f.strip_prefix("CH").and_then(|c| c.parse::<u8>().ok()) {
            Some(c @ 1..=4) => sig::tim(periph.as_bytes()[3] - b'0', c - 1),
            _ => sig::NONE,
        },
        _ => sig::NONE,
    }
}

/// Clock frequencies and bus ratios derived from RCC.
#[derive(Clone, Copy, Debug)]
pub struct ClockTree {
    pub sysclk_hz: f64,
    pub hclk_hz: f64,
    /// APB1 / APB2 prescaler divisors (1, 2, 4, 8, 16).
    pub ppre1: u32,
    pub ppre2: u32,
}

impl ClockTree {
    /// Ratio HCLK cycles per timer-kernel clock cycle on an APB bus: timers run at PCLK, or at
    /// 2 x PCLK when the bus prescaler is not 1.
    pub fn timer_div(&self, apb: u8) -> u32 {
        let p = if apb == 2 { self.ppre2 } else { self.ppre1 };
        if p == 1 { 1 } else { p / 2 }
    }

    /// HCLK cycles per PCLK cycle of an APB bus.
    pub fn pclk_div(&self, apb: u8) -> u32 {
        if apb == 2 { self.ppre2 } else { self.ppre1 }
    }
}

/// Electrical configuration of one GPIO as written by the port registers.
#[derive(Clone, Copy, Debug, Default)]
pub struct PinCfg {
    /// MODER: 0 input, 1 output, 2 alternate function, 3 analog.
    pub mode: u8,
    pub open_drain: bool,
    pub odr: u8,
    pub pullup: bool,
    pub pulldown: bool,
}

pub struct ArmSys {
    pub pins: Vec<Pin>,
    pub pcfg: Vec<PinCfg>,
    levels: Vec<u32>,
    pub trace: PinTrace,
    pub clock: ClockModel,
    pub clk: ClockTree,
    pub vcc: f64,
    /// Crystal / external clock frequency feeding HSE.
    pub hse_hz: f64,
    /// Bytes decoded by the Serial Monitor since the last snapshot.
    pub serial_out: Vec<u8>,
    pub messages: Vec<Message>,
    warned: HashSet<String>,
    /// RCC peripheral clock enable registers mirrored for clock gating: AHB1, AHB2, AHB3, APB1 low,
    /// APB1 high, APB2.
    pub enr: [u32; 6],
    pub flash_latency: u8,
    /// PWR_CR5.R1MODE == 0: Range 1 boost mode.
    pub boost: bool,
    /// Alternate-function table: pin -> AF number -> signal id.
    pub af: Vec<[u16; 16]>,
    /// Signal currently selected on each pin (0 = none or not in alternate-function mode).
    pub pin_sig: Vec<u16>,
    /// Pin currently routing each signal.
    pub route: [u16; NSIG],
    /// Output level each routed output signal wants to drive.
    pub sig_level: [u8; NSIG],
    /// Pin level changes not yet delivered to the listening peripherals: (pin, level, cycle).
    pub changed: Vec<(u16, u8, u64)>,
    /// Peripherals (event-owner indices) receiving [`Mmio::on_pin`](super::bus::Mmio::on_pin).
    pub listeners: Vec<u8>,
    /// Peripherals reset through RCC_xRSTR: (register, bit).
    pub resets: Vec<(u8, u8)>,
    /// Something needs the machine's attention after the current bus access (pin changes, clock
    /// changes, peripheral resets).
    pub attn: bool,
    pub clock_dirty: bool,
}

impl ArmSys {
    pub fn new(gpio_count: usize, hsi_hz: f64, vcc: f64, hse_hz: f64) -> Self {
        let words = gpio_count.div_ceil(32).max(1);
        let mut sig_level = [0u8; NSIG];
        for p in 1..=6u8 {
            sig_level[sig::uart(p, false) as usize] = 1; // idle UART lines are high
        }
        Self {
            pins: (0..gpio_count).map(|i| Pin::new(format!("P{}{}", (b'A' + (i / 16) as u8) as char, i % 16))).collect(),
            pcfg: vec![PinCfg::default(); gpio_count],
            levels: vec![0; words],
            trace: PinTrace::new(1 << 16, gpio_count),
            clock: ClockModel::new(hsi_hz),
            clk: ClockTree { sysclk_hz: hsi_hz, hclk_hz: hsi_hz, ppre1: 1, ppre2: 1 },
            vcc,
            hse_hz,
            serial_out: Vec::new(),
            messages: Vec::new(),
            warned: HashSet::new(),
            enr: [0; 6],
            flash_latency: 0,
            boost: false,
            af: vec![[sig::NONE; 16]; gpio_count],
            pin_sig: vec![sig::NONE; gpio_count],
            route: [NO_PIN; NSIG],
            sig_level,
            changed: Vec::new(),
            listeners: Vec::new(),
            resets: Vec::new(),
            attn: false,
            clock_dirty: false,
        }
    }

    // ---- messages -------------------------------------------------------------------------

    pub fn message(&mut self, cycle: u64, level: &'static str, text: impl Into<String>) {
        self.messages.push(Message { cycle, level, text: text.into() });
        if self.messages.len() > 200 {
            let excess = self.messages.len() - 200;
            self.messages.drain(..excess);
        }
    }

    /// De-duplicated warning (tight loops would otherwise flood the log).
    pub fn warn_key(&mut self, cycle: u64, key: impl Into<String>, text: impl Into<String>) {
        let key = key.into();
        if self.warned.contains(&key) {
            return;
        }
        if self.warned.len() > 1000 {
            self.warned.clear();
        }
        self.warned.insert(key);
        self.message(cycle, "warning", text);
    }

    // ---- clock gating and clock tree ------------------------------------------------------

    /// True when the RCC clock-enable bit `bit` of enable register `reg` is set.
    #[inline]
    pub fn clock_on(&self, reg: u8, bit: u8) -> bool {
        self.enr[reg as usize] >> bit & 1 != 0
    }

    /// Applies new clock-tree values at cycle `now`. Peripherals are told through
    /// `on_clock_change` once the current bus access completes.
    pub fn set_clock_tree(&mut self, t: ClockTree, now: u64) {
        let changed = self.clk.hclk_hz != t.hclk_hz || self.clk.ppre1 != t.ppre1 || self.clk.ppre2 != t.ppre2;
        self.clk = t;
        self.clock.set_hz(t.hclk_hz, now);
        if changed {
            self.clock_dirty = true;
            self.attn = true;
        }
    }

    pub fn time_at(&self, cycle: u64) -> f64 {
        self.clock.time_at(cycle)
    }

    // ---- pins -----------------------------------------------------------------------------

    /// Re-resolves the level of pin `i` after any change of its inputs; records the trace and
    /// queues a pin event when the level changed.
    pub fn update_pin(&mut self, i: usize, cycle: u64) {
        let vcc = self.vcc;
        let p = &mut self.pins[i];
        if !p.resolve(vcc) {
            return;
        }
        let lv = p.level;
        let (w, b) = (i >> 5, i & 31);
        self.levels[w] = (self.levels[w] & !(1 << b)) | (lv as u32) << b;
        self.trace.record(cycle, &self.levels);
        self.changed.push((i as u16, lv, cycle));
        self.attn = true;
    }

    /// Recomputes the MCU-side drive of pin `i` from the port registers (`pcfg`), the selected
    /// alternate function and the signal level, then re-resolves the pin.
    pub fn refresh_pin(&mut self, i: usize, cycle: u64) {
        let c = self.pcfg[i];
        let sig = if c.mode == 2 { self.pin_sig[i] } else { sig::NONE };
        let level = self.sig_level[sig as usize];
        let p = &mut self.pins[i];
        let driven = match c.mode {
            1 => {
                p.dir = 1;
                p.ov_enable = 0;
                p.out = c.odr;
                c.odr
            }
            2 if sig != sig::NONE && !sig::is_input(sig) => {
                // Alternate function of a modelled peripheral. Unmodelled alternate functions
                // leave the pin high-impedance.
                p.dir = 1;
                p.ov_enable = 1;
                p.ov_value = level;
                level
            }
            _ => {
                p.dir = 0;
                p.ov_enable = 0;
                0
            }
        };
        // Open-drain outputs only pull low; a "1" releases the pin.
        p.ddoe = (c.open_drain && p.dir == 1 && driven == 1) as u8;
        p.ddov = 0;
        p.pullup = c.pullup as u8;
        p.pulldown = c.pulldown as u8;
        self.update_pin(i, cycle);
    }

    /// Selects alternate function `af` (or none) on pin `i`; call [`ArmSys::refresh_pin`] afterwards.
    pub fn select_af(&mut self, i: usize, af: u8) {
        let new = if self.pcfg[i].mode == 2 { self.af[i][(af & 15) as usize] } else { sig::NONE };
        let old = std::mem::replace(&mut self.pin_sig[i], new);
        if old != sig::NONE && self.route[old as usize] == i as u16 {
            self.route[old as usize] = NO_PIN;
        }
        if new != sig::NONE {
            self.route[new as usize] = i as u16;
        }
    }

    /// Drives output signal `sig` to `level`, on the pin that routes it.
    pub fn sig_out(&mut self, sig: u16, level: u8, cycle: u64) {
        let s = sig as usize;
        if self.sig_level[s] == level {
            return;
        }
        self.sig_level[s] = level;
        let pin = self.route[s];
        if pin != NO_PIN {
            self.refresh_pin(pin as usize, cycle);
        }
    }

    /// The pin that currently routes `sig`, if any.
    #[inline]
    pub fn pin_of(&self, sig: u16) -> Option<usize> {
        let p = self.route[sig as usize];
        (p != NO_PIN).then_some(p as usize)
    }

    /// Resets the GPIO electrical state and the signal routing (power-on / system reset). Inputs
    /// from the outside world (`ext`, generators) are kept.
    pub fn reset_pins(&mut self, cycle: u64) {
        self.route = [NO_PIN; NSIG];
        self.pin_sig.iter_mut().for_each(|s| *s = sig::NONE);
        for p in 1..=6u8 {
            self.sig_level[sig::uart(p, false) as usize] = 1;
            self.sig_level[sig::uart(p, true) as usize] = 0;
        }
        for t in 2..=4u8 {
            for c in 0..4 {
                self.sig_level[sig::tim(t, c) as usize] = 0;
            }
        }
        for i in 0..self.pins.len() {
            self.pcfg[i] = PinCfg::default();
            self.refresh_pin(i, cycle);
        }
    }

    /// Clears everything that belongs to the previous run (power cycle).
    pub fn power_on(&mut self, hsi_hz: f64) {
        self.trace.clear();
        self.levels.iter_mut().for_each(|w| *w = 0);
        for p in self.pins.iter_mut() {
            p.level = 0;
            p.volts = 0.0;
        }
        self.clock.reset(hsi_hz);
        self.clk = ClockTree { sysclk_hz: hsi_hz, hclk_hz: hsi_hz, ppre1: 1, ppre2: 1 };
        self.enr = [0; 6];
        self.flash_latency = 0;
        self.boost = false;
        self.changed.clear();
        self.resets.clear();
        self.messages.clear();
        self.warned.clear();
        self.serial_out.clear();
        self.attn = false;
        self.clock_dirty = false;
    }
}
