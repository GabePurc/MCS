//! GPIO matrix, IO MUX and the GPIO peripheral registers (ESP32-C3 TRM "IO MUX and GPIO Matrix"; register
//! layout from the SVD).
//!
//! Each of the 22 pads has an IO MUX register (function select `MCU_SEL`, pull-up / pull-down, input
//! enable) and GPIO matrix registers (`FUNCn_OUT_SEL_CFG`: which peripheral output signal or the plain
//! `GPIO_OUT` bit drives the pad; `OEN_SEL`: where the output enable comes from). Peripheral input signals
//! are routed to a pad by `FUNCx_IN_SEL_CFG`.
//!
//! Modelled: IO MUX function 1 (GPIO, through the matrix) on every pad and function 0 on all pads except
//! GPIO20 / GPIO21, where function 0 is U0RXD / U0TXD, connected directly to UART0 (the reset state of
//! those two pads); other functions (SPI, JTAG, USB) leave the pad high-impedance. Output signals: the TX
//! lines of UART0 (signal 6) and UART1 (signal 9); input signals: RX of UART0 (6) and UART1 (9), others
//! read as 1 when unrouted. `OUT_SEL = 0x80` is the plain GPIO output; the output enable then always comes
//! from `GPIO_ENABLE`. GPIO interrupts: `PINn.INT_TYPE` (1 rising, 2 falling, 3 any edge, 4 low level, 5
//! high level) with `INT_ENA` bit 0 (CPU interrupt) feeding interrupt matrix source 16.
//!
//! Assumptions: `GPIO_STRAP_REG` bit n holds the level GPIOn had at reset (GPIO2, GPIO8, GPIO9); matrix input
//! selects `IN_SEL >= 22` read a constant (bit 3 of the 6-bit value used by the ROM headers: 0x30 -> 0,
//! 0x38 -> 1); the IO MUX reset pull-up (`FUN_WPU`) of every pad follows the SVD reset value (0xb00).

use crate::riscv::bus::{Cx, Mmio};

use super::sys::{Sys, NGPIO};

/// GPIO matrix output signal numbers (ESP32-C3 TRM table "Peripheral signals via GPIO matrix").
pub const SIG_U0TXD: usize = 6;
pub const SIG_U1TXD: usize = 9;
pub const SIG_U0RXD: usize = 6;
pub const SIG_U1RXD: usize = 9;
/// `OUT_SEL` value selecting the plain GPIO output register.
pub const OUT_SEL_GPIO: u32 = 0x80;

const SRC_GPIO: u8 = 16;

pub struct GpioState {
    pub out: u32,
    pub enable: u32,
    /// Interrupt status (one bit per pad).
    pub status: u32,
    pub strap: u32,
    pub pin_cfg: [u32; NGPIO],
    pub out_sel: [u32; NGPIO],
    pub in_sel: [u32; 128],
    /// IO MUX pad registers.
    pub mux: [u32; NGPIO],
    pub pin_ctrl: u32,
    /// Level every peripheral output signal drives (index = signal number).
    pub sig_level: [u8; 128],
    /// Peripheral output signal has its output enabled.
    pub sig_oe: [bool; 128],
    /// Other registers (BT_SELECT, SDIO_SELECT, CLOCK_GATE...) by offset / 4.
    misc: [u32; 0x1c0],
}

const MUX_RESET: u32 = 0x0000_0b00;

impl GpioState {
    pub fn new() -> Self {
        let mut s = Self {
            out: 0,
            enable: 0,
            status: 0,
            strap: 0,
            pin_cfg: [0; NGPIO],
            out_sel: [OUT_SEL_GPIO; NGPIO],
            in_sel: [0; 128],
            mux: [MUX_RESET; NGPIO],
            pin_ctrl: 0x7ff,
            sig_level: [0; 128],
            sig_oe: [false; 128],
            misc: [0; 0x1c0],
        };
        s.reset_signals();
        s.misc[0x62c / 4] = 1; // CLOCK_GATE
        s
    }

    fn reset_signals(&mut self) {
        self.sig_level = [0; 128];
        self.sig_oe = [false; 128];
        for s in [SIG_U0TXD, SIG_U1TXD] {
            self.sig_level[s] = 1; // idle UART lines are high
            self.sig_oe[s] = true;
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

impl Default for GpioState {
    fn default() -> Self {
        Self::new()
    }
}

impl Sys {
    /// Recomputes the electrical drive of pad `i` from the IO MUX and GPIO matrix configuration.
    pub fn gpio_refresh(&mut self, i: usize, cycle: u64) {
        let g = &self.gpio;
        let m = g.mux[i];
        let sel = (m >> 12) & 7;
        let (oe, val) = if sel == 0 && (i == 20 || i == 21) {
            // Function 0 of GPIO21 is U0TXD (direct), GPIO20 is the U0RXD input.
            (i == 21, g.sig_level[SIG_U0TXD] != 0)
        } else if sel <= 1 {
            let cfg = g.out_sel[i];
            let osel = cfg & 0xff;
            let inv = cfg >> 8 & 1 != 0;
            let plain = osel == OUT_SEL_GPIO;
            let v = (if plain { g.out >> i & 1 != 0 } else { g.sig_level[(osel & 127) as usize] != 0 }) ^ inv;
            let oe = if plain || cfg >> 9 & 1 != 0 { (g.enable >> i & 1 != 0) ^ (cfg >> 10 & 1 != 0 && !plain) } else { g.sig_oe[(osel & 127) as usize] };
            (oe, v)
        } else {
            (false, false)
        };
        let out = (g.out >> i & 1) as u8;
        let p = &mut self.pins[i];
        p.dir = oe as u8;
        p.out = out;
        p.ov_enable = 1;
        p.ov_value = val as u8;
        p.ddoe = 0;
        p.pullup = (m >> 8 & 1) as u8;
        p.pulldown = (m >> 7 & 1) as u8;
        self.update_pin(i, cycle);
    }

    pub fn gpio_refresh_all(&mut self, cycle: u64) {
        for i in 0..self.pins.len().min(NGPIO) {
            self.gpio_refresh(i, cycle);
        }
    }

    /// Drives peripheral output signal `sig` to `level` and refreshes the pads that route it.
    pub fn sig_out(&mut self, sig: usize, level: u8, cycle: u64) {
        if self.gpio.sig_level[sig] == level {
            return;
        }
        self.gpio.sig_level[sig] = level;
        if sig == SIG_U0TXD && self.gpio.mux[21] >> 12 & 7 == 0 {
            self.gpio_refresh(21, cycle);
        }
        for i in 0..NGPIO.min(self.pins.len()) {
            if self.gpio.mux[i] >> 12 & 7 <= 1 && self.gpio.out_sel[i] & 0xff == sig as u32 {
                self.gpio_refresh(i, cycle);
            }
        }
    }

    /// The pad that currently feeds peripheral input signal `sig`, if any.
    pub fn input_pin(&self, sig: usize) -> Option<usize> {
        let cfg = self.gpio.in_sel[sig & 127];
        if cfg >> 6 & 1 != 0 {
            let p = (cfg & 0x1f) as usize;
            return (p < NGPIO).then_some(p);
        }
        (sig == SIG_U0RXD && self.gpio.mux[20] >> 12 & 7 == 0).then_some(20)
    }

    /// Logic level seen by peripheral input signal `sig` (1 when nothing is routed: idle UART).
    pub fn input_level(&self, sig: usize) -> u8 {
        let cfg = self.gpio.in_sel[sig & 127];
        let lv = match self.input_pin(sig) {
            Some(p) => self.pins[p].level,
            None if cfg >> 6 & 1 != 0 => (cfg >> 3 & 1) as u8,
            None => 1,
        };
        lv ^ (cfg >> 5 & 1) as u8
    }

    /// Latches / evaluates the GPIO interrupt condition of pad `i` after a level change.
    pub fn gpio_pin_changed(&mut self, i: usize, level: u8, cycle: u64) {
        if i >= NGPIO {
            return;
        }
        let ty = self.gpio.pin_cfg[i] >> 7 & 7;
        let hit = matches!((ty, level), (1, 1) | (2, 0) | (3, _) | (4, 0) | (5, 1));
        if hit {
            self.gpio.status |= 1 << i;
        }
        let _ = cycle;
    }

    /// Re-evaluates level-type interrupt conditions of every pad (after configuration changes).
    pub fn gpio_level_scan(&mut self) {
        for i in 0..NGPIO.min(self.pins.len()) {
            let ty = self.gpio.pin_cfg[i] >> 7 & 7;
            let l = self.pins[i].level;
            if (ty == 4 && l == 0) || (ty == 5 && l == 1) {
                self.gpio.status |= 1 << i;
            }
        }
    }

    /// Pads whose interrupt reaches the CPU (`PINn.INT_ENA` bit 0).
    pub fn gpio_irq_mask(&self) -> u32 {
        let mut m = 0;
        for i in 0..NGPIO {
            if self.gpio.pin_cfg[i] >> 13 & 1 != 0 {
                m |= 1 << i;
            }
        }
        m
    }

    /// Level of interrupt matrix source 16 (GPIO).
    pub fn gpio_irq_on(&self) -> bool {
        self.gpio.status & self.gpio_irq_mask() != 0
    }
}

impl Cx {
    /// Pushes the GPIO interrupt status to the interrupt matrix.
    pub fn gpio_irq_sync(&mut self) {
        let on = self.sys.gpio_irq_on();
        self.irq_source(SRC_GPIO, on);
    }
}

// The GPIO register block (0x60004000, 4 KiB; the GPIO_SD block at +0xf00 reads as zero).
pub struct Gpio;

impl Gpio {
    pub fn new() -> Self {
        Self
    }

    fn input_reg(sys: &Sys) -> u32 {
        let mut v = 0;
        for i in 0..NGPIO.min(sys.pins.len()) {
            if sys.gpio.mux[i] >> 9 & 1 != 0 && sys.pins[i].level != 0 {
                v |= 1 << i;
            }
        }
        v
    }

    fn refresh_mask(cx: &mut Cx, mask: u32) {
        let c = cx.cycles;
        for i in 0..NGPIO {
            if mask >> i & 1 != 0 {
                cx.sys.gpio_refresh(i, c);
            }
        }
    }
}

impl Default for Gpio {
    fn default() -> Self {
        Self::new()
    }
}

const ALL: u32 = (1 << NGPIO) - 1;

impl Mmio for Gpio {
    fn read(&mut self, off: u32, _size: u8, cx: &mut Cx) -> u32 {
        let g = &cx.sys.gpio;
        match off {
            0x04 => g.out,
            0x20 => g.enable,
            0x38 => g.strap,
            0x3c => Self::input_reg(&cx.sys),
            0x44 | 0x14c => g.status,
            0x5c => g.status & cx.sys.gpio_irq_mask(),
            0x74..=0xc8 => g.pin_cfg[((off - 0x74) / 4) as usize],
            0x154..=0x350 => g.in_sel[((off - 0x154) / 4) as usize],
            0x554..=0x5a8 => g.out_sel[((off - 0x554) / 4) as usize],
            0xf00..=0xfff => 0,
            _ => g.misc_read(off),
        }
    }

    fn write(&mut self, off: u32, _size: u8, v: u32, cx: &mut Cx) {
        let c = cx.cycles;
        match off {
            0x04 => {
                let d = cx.sys.gpio.out ^ v;
                cx.sys.gpio.out = v & ALL;
                Self::refresh_mask(cx, d & ALL);
            }
            0x08 | 0x0c => {
                let old = cx.sys.gpio.out;
                let new = if off == 0x08 { old | v } else { old & !v } & ALL;
                cx.sys.gpio.out = new;
                Self::refresh_mask(cx, old ^ new);
            }
            0x20 | 0x24 | 0x28 => {
                let old = cx.sys.gpio.enable;
                let new = match off {
                    0x20 => v,
                    0x24 => old | v,
                    _ => old & !v,
                } & ALL;
                cx.sys.gpio.enable = new;
                Self::refresh_mask(cx, old ^ new);
            }
            0x44 | 0x48 | 0x4c => {
                let g = &mut cx.sys.gpio;
                match off {
                    0x44 => g.status = v & ALL,
                    0x48 => g.status |= v & ALL,
                    _ => g.status &= !v,
                }
                cx.sys.gpio_level_scan();
                cx.gpio_irq_sync();
            }
            0x74..=0xc8 => {
                cx.sys.gpio.pin_cfg[((off - 0x74) / 4) as usize] = v;
                cx.sys.gpio_level_scan();
                cx.gpio_irq_sync();
            }
            0x154..=0x350 => {
                cx.sys.gpio.in_sel[((off - 0x154) / 4) as usize] = v & 0x7f;
            }
            0x554..=0x5a8 => {
                let i = ((off - 0x554) / 4) as usize;
                cx.sys.gpio.out_sel[i] = v & 0x7ff;
                cx.sys.gpio_refresh(i, c);
            }
            0x38 | 0x3c | 0x5c | 0x14c | 0xf00..=0xfff => {}
            _ => cx.sys.gpio.misc_write(off, v),
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        let strap = cx.sys.gpio.strap;
        cx.sys.gpio.reset();
        cx.sys.gpio.strap = strap;
        let c = cx.cycles;
        cx.sys.gpio_refresh_all(c);
        cx.gpio_irq_sync();
    }

    fn inspect(&self, cx: &Cx) -> Vec<(String, String)> {
        let g = &cx.sys.gpio;
        vec![("GPIO_OUT".into(), format!("{:#08x}", g.out)), ("GPIO_ENABLE".into(), format!("{:#08x}", g.enable)), ("GPIO_IN".into(), format!("{:#08x}", Gpio::input_reg(&cx.sys)))]
    }
}

impl GpioState {
    fn misc_read(&self, off: u32) -> u32 {
        self.misc.get((off / 4) as usize).copied().unwrap_or(0)
    }

    fn misc_write(&mut self, off: u32, v: u32) {
        if let Some(m) = self.misc.get_mut((off / 4) as usize) {
            *m = v;
        }
    }
}

// The IO MUX register block (0x60009000).
pub struct IoMux;

impl Mmio for IoMux {
    fn read(&mut self, off: u32, _size: u8, cx: &mut Cx) -> u32 {
        match off {
            0x00 => cx.sys.gpio.pin_ctrl,
            0x04..=0x58 => cx.sys.gpio.mux[((off - 4) / 4) as usize],
            0xfc => 0x0200_6050,
            _ => 0,
        }
    }

    fn write(&mut self, off: u32, _size: u8, v: u32, cx: &mut Cx) {
        match off {
            0x00 => cx.sys.gpio.pin_ctrl = v & 0xfff,
            0x04..=0x58 => {
                let i = ((off - 4) / 4) as usize;
                cx.sys.gpio.mux[i] = v & 0xffff;
                let c = cx.cycles;
                cx.sys.gpio_refresh(i, c);
                cx.sys.gpio_level_scan();
            }
            _ => {}
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        cx.sys.gpio.mux = [MUX_RESET; NGPIO];
        cx.sys.gpio.pin_ctrl = 0x7ff;
        let c = cx.cycles;
        cx.sys.gpio_refresh_all(c);
    }
}
