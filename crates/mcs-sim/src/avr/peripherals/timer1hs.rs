//! ATtiny25/45/85 Timer/Counter1: 8-bit high-speed timer clocked by the system clock or the
//! 64 MHz PLL (32 MHz in low-speed mode), prescaler /1 .. /16384, OCR1C as TOP in CTC and PWM
//! modes, two compare units with PWM and complementary (inverted) outputs, force compare.
//!
//! Ticks are tracked in time (the PLL clock is asynchronous to the CPU clock), so the counter
//! is advanced lazily like the other timers and events are scheduled at observable ticks only.
//! The dead-time generator (DTPS1/DT1A/DT1B) is not modelled.
//!
//! Source: Atmel-2586Q section 12 (Timer/Counter1).

use crate::avr::machine::{Cx, Peripheral, Trigger};
use crate::avr::peripherals::irqflags::update_irqs;

#[derive(Clone)]
pub struct Timer1HsConfig {
    pub tccr1: u16,
    pub gtccr: u16,
    pub tcnt1: u16,
    pub ocr1a: u16,
    pub ocr1b: u16,
    pub ocr1c: u16,
    pub pllcsr: u16,
    pub tifr: u16,
    pub timsk: u16,
    /// TIFR/TIMSK bits: OCF1A, OCF1B, TOV1.
    pub ocfa: u8,
    pub ocfb: u8,
    pub tov: u8,
    pub v_comp_a: u8,
    pub v_comp_b: u8,
    pub v_ovf: u8,
    /// OC1A, !OC1A, OC1B, !OC1B GPIOs.
    pub oc: [usize; 4],
    pub prescaler_group: u8,
    pub prr_mask: u16,
}

const CTC1: u8 = 0x80;
const PWM1A: u8 = 0x40;
const PWM1B: u8 = 0x40; // in GTCCR
const PCKE: u8 = 0x04;
const PLOCK: u8 = 0x01;
const LSM: u8 = 0x80;
const EV_TICK: u8 = 0;

pub struct Timer1Hs {
    c: Timer1HsConfig,
    irq_map: [(u8, u8); 3],
    tcnt: u32,
    ocr_a: u32,
    ocr_b: u32,
    ocr_c: u32,
    tccr1: u8,
    /// OC1A, OC1B output values.
    oc: [u8; 2],
    /// Time origin of the tick grid and ticks processed since then.
    t0: f64,
    done: u64,
    tsm: bool,
    halted: bool,
    power_reduced: bool,
}

impl Timer1Hs {
    pub fn new(c: Timer1HsConfig) -> Self {
        let irq_map = [(c.ocfa, c.v_comp_a), (c.ocfb, c.v_comp_b), (c.tov, c.v_ovf)];
        Self { c, irq_map, tcnt: 0, ocr_a: 0, ocr_b: 0, ocr_c: 0, tccr1: 0, oc: [0; 2], t0: 0.0, done: 0, tsm: false, halted: false, power_reduced: false }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        let c = &self.c;
        vec![(c.tccr1, 0), (c.tcnt1, 0), (c.ocr1a, 0), (c.ocr1b, 0), (c.ocr1c, 0)]
    }

    pub fn irq_map(&self) -> Vec<(u8, u8)> {
        self.irq_map.to_vec()
    }

    fn gtccr(&self, cx: &Cx) -> u8 {
        cx.cpu.data[self.c.gtccr as usize]
    }

    fn pwm(&self, ch: usize, cx: &Cx) -> bool {
        if ch == 0 { self.tccr1 & PWM1A != 0 } else { self.gtccr(cx) & PWM1B != 0 }
    }

    fn com(&self, ch: usize, cx: &Cx) -> u8 {
        if ch == 0 { (self.tccr1 >> 4) & 3 } else { (self.gtccr(cx) >> 4) & 3 }
    }

    fn top(&self, cx: &Cx) -> u32 {
        if self.tccr1 & CTC1 != 0 || self.pwm(0, cx) || self.pwm(1, cx) { self.ocr_c } else { 0xff }
    }

    /// Timer clock (Hz) after the prescaler, 0 when stopped.
    fn tick_hz(&self, cx: &Cx) -> f64 {
        let cs = self.tccr1 & 0x0f;
        if cs == 0 || self.tsm || self.halted || self.power_reduced {
            return 0.0;
        }
        let pll = cx.cpu.data[self.c.pllcsr as usize];
        let src = if pll & PCKE != 0 && pll & PLOCK != 0 { if pll & LSM != 0 { 32e6 } else { 64e6 } } else { cx.sys.clock.hz };
        src / (1u64 << (cs - 1)) as f64
    }

    fn distance(&self, cx: &Cx) -> u64 {
        let c = self.tcnt;
        let top = self.top(cx);
        let lim = if c <= top { top } else { 0xff };
        let mut d = lim - c + 1;
        for o in [self.ocr_a, self.ocr_b] {
            if o >= c && o <= lim {
                d = d.min(o - c + 1);
            }
        }
        d as u64
    }

    fn tick_once(&mut self, cycle: u64, cx: &mut Cx) {
        let old = self.tcnt;
        let top = self.top(cx);
        let mut flags = 0;
        if old == self.ocr_a {
            flags |= self.c.ocfa;
            self.compare(0, cycle, cx);
        }
        if old == self.ocr_b {
            flags |= self.c.ocfb;
            self.compare(1, cycle, cx);
        }
        if old == top || old == 0xff {
            self.tcnt = 0;
            flags |= self.c.tov;
            self.bottom(cycle, cx);
            cx.sys.trigger(Trigger::TimerOvf(1), 1, cycle);
        } else {
            self.tcnt = old + 1;
        }
        if flags != 0 {
            cx.cpu.data[self.c.tifr as usize] |= flags;
            update_irqs(cx, self.c.tifr, self.c.timsk, &self.irq_map);
        }
    }

    fn compare(&mut self, ch: usize, cycle: u64, cx: &mut Cx) {
        let com = self.com(ch, cx);
        let v = if self.pwm(ch, cx) {
            match com {
                1 | 2 => 0,
                3 => 1,
                _ => return,
            }
        } else {
            match com {
                1 => self.oc[ch] ^ 1,
                2 => 0,
                3 => 1,
                _ => return,
            }
        };
        self.oc[ch] = v;
        self.apply_outputs(cycle, cx);
    }

    fn bottom(&mut self, cycle: u64, cx: &mut Cx) {
        for ch in 0..2 {
            if self.pwm(ch, cx) {
                match self.com(ch, cx) {
                    1 | 2 => self.oc[ch] = 1,
                    3 => self.oc[ch] = 0,
                    _ => {}
                }
            }
        }
        self.apply_outputs(cycle, cx);
    }

    /// OC1x and complementary !OC1x pin overrides (Atmel-2586Q tables 12-1/12-2).
    fn apply_outputs(&mut self, cycle: u64, cx: &mut Cx) {
        for ch in 0..2 {
            let com = self.com(ch, cx);
            let pwm = self.pwm(ch, cx);
            let pins = [(self.c.oc[ch * 2], com != 0, self.oc[ch]), (self.c.oc[ch * 2 + 1], pwm && com == 1, self.oc[ch] ^ 1)];
            for (g, en, val) in pins {
                let p = &mut cx.sys.pins[g];
                let en = en as u8;
                if p.ov_enable != en || (en == 1 && p.ov_value != val) {
                    p.ov_enable = en;
                    p.ov_value = val;
                    cx.sys.update_pin(g, cycle);
                }
            }
        }
    }

    /// Advances the counter to the current time.
    fn sync(&mut self, cx: &mut Cx) {
        let hz = self.tick_hz(cx);
        if hz <= 0.0 {
            return;
        }
        let t = cx.time_seconds();
        let target = ((t - self.t0) * hz + 1e-9).floor().max(0.0) as u64;
        while self.done < target {
            let d = self.distance(cx);
            let left = target - self.done;
            if d > left {
                self.tcnt = (self.tcnt + left as u32) & 0xff;
                self.done = target;
                break;
            }
            self.tcnt = (self.tcnt + d as u32 - 1) & 0xff;
            self.done += d;
            let at = cx.sys.clock.cycle_at(self.t0 + self.done as f64 / hz);
            self.tick_once(at, cx);
        }
    }

    /// Restarts the tick grid now (clock source / prescaler / CPU clock changed).
    fn rebase(&mut self, cx: &mut Cx) {
        self.t0 = cx.time_seconds();
        self.done = 0;
    }

    fn schedule(&mut self, cx: &mut Cx) {
        let hz = self.tick_hz(cx);
        if hz <= 0.0 {
            cx.cancel(EV_TICK);
            return;
        }
        let k = self.done + self.distance(cx);
        let at = cx.sys.clock.cycle_at(self.t0 + k as f64 / hz).max(cx.now() + 1);
        cx.schedule(EV_TICK, at);
    }

    fn reconfigure(&mut self, cx: &mut Cx) {
        self.sync(cx);
        self.rebase(cx);
        let now = cx.now();
        self.apply_outputs(now, cx);
        self.schedule(cx);
    }

    fn reg_value(&self, addr: u16) -> Option<u32> {
        let c = &self.c;
        if addr == c.tcnt1 {
            Some(self.tcnt)
        } else if addr == c.ocr1a {
            Some(self.ocr_a)
        } else if addr == c.ocr1b {
            Some(self.ocr_b)
        } else if addr == c.ocr1c {
            Some(self.ocr_c)
        } else {
            None
        }
    }
}

impl Peripheral for Timer1Hs {
    fn name(&self) -> &str {
        "TC1"
    }

    fn read(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        self.peek(addr, cx)
    }

    fn peek(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        self.sync(cx);
        self.reg_value(addr).map(|v| v as u8).unwrap_or(self.tccr1)
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        self.sync(cx);
        let c = &self.c;
        if addr == c.tccr1 {
            self.tccr1 = v;
        } else if addr == c.tcnt1 {
            self.tcnt = v as u32;
        } else if addr == c.ocr1a {
            self.ocr_a = v as u32;
        } else if addr == c.ocr1b {
            self.ocr_b = v as u32;
        } else if addr == c.ocr1c {
            self.ocr_c = v as u32;
        }
        self.reconfigure(cx);
    }

    fn on_event(&mut self, _tag: u8, _cycle: u64, cx: &mut Cx) {
        self.sync(cx);
        self.schedule(cx);
    }

    fn on_reg_written(&mut self, addr: u16, cx: &mut Cx) {
        if addr == self.c.pllcsr || addr == self.c.gtccr {
            self.reconfigure(cx);
        }
    }

    fn on_trigger(&mut self, trigger: Trigger, value: u8, _cycle: u64, cx: &mut Cx) {
        match trigger {
            Trigger::TimerSync => {
                self.sync(cx);
                self.tsm = value != 0;
                self.reconfigure(cx);
            }
            Trigger::PrescalerReset if value & self.c.prescaler_group != 0 => self.reconfigure(cx),
            Trigger::GtccrStrobe => {
                // FOC1A (bit 2) / FOC1B (bit 3) force a compare match in non-PWM mode.
                let now = cx.now();
                for (ch, bit) in [(0usize, 0x04u8), (1, 0x08)] {
                    if value & bit != 0 && !self.pwm(ch, cx) {
                        self.compare(ch, now, cx);
                    }
                }
            }
            _ => {}
        }
    }

    fn on_clock_change(&mut self, cx: &mut Cx) {
        self.reconfigure(cx);
    }

    fn on_power_reduction(&mut self, prr: u16, cx: &mut Cx) {
        self.sync(cx);
        self.power_reduced = prr & self.c.prr_mask != 0;
        self.reconfigure(cx);
    }

    fn on_sleep(&mut self, mode: u8, cx: &mut Cx) {
        if mode != 0 {
            self.sync(cx);
            self.halted = true;
            self.reconfigure(cx);
        }
    }

    fn on_wake(&mut self, cx: &mut Cx) {
        if self.halted {
            self.halted = false;
            self.reconfigure(cx);
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        cx.cancel(EV_TICK);
        *self = Timer1Hs::new(self.c.clone());
        self.rebase(cx);
        let now = cx.now();
        self.apply_outputs(now, cx);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        self.sync(cx);
        let cs = self.tccr1 & 0x0f;
        let pll = cx.cpu.data[self.c.pllcsr as usize] & PCKE != 0;
        vec![
            ("Clock".into(), if cs == 0 { "Stopped".into() } else { format!("{}/{}", if pll { "PCK" } else { "CK" }, 1u32 << (cs - 1)) }),
            ("TCNT1".into(), self.tcnt.to_string()),
            ("TOP".into(), self.top(cx).to_string()),
            ("Mode".into(), if self.pwm(0, cx) || self.pwm(1, cx) { "PWM (TOP=OCR1C)" } else if self.tccr1 & CTC1 != 0 { "CTC (TOP=OCR1C)" } else { "Normal" }.into()),
            ("OC1A / OC1B".into(), format!("{} / {}", self.oc[0], self.oc[1])),
        ]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
