//! 16-bit Timer/Counter with all 16 waveform generation modes (normal, CTC, fast PWM, phase
//! correct and phase & frequency correct PWM), two output compare units with pin outputs,
//! input capture, external clock input and the shared TEMP register for 16-bit access.
//!
//! Event driven: the counter is advanced lazily ("synced") to the current cycle whenever
//! software touches a register, and one scheduler event is armed for the next timer tick where
//! something observable happens (compare match, TOP, BOTTOM, MAX). Between those ticks the count
//! is linear, so syncing costs O(1) per event regardless of how many ticks elapsed.

use crate::avr::machine::{Cx, Peripheral, Trigger};

#[derive(Clone)]
pub struct Timer16Config {
    pub name: &'static str,
    pub tccr_a: u16,
    pub tccr_b: u16,
    pub tccr_c: u16,
    pub tcnt_l: u16,
    pub tcnt_h: u16,
    pub ocr_a_l: u16,
    pub ocr_a_h: u16,
    pub ocr_b_l: u16,
    pub ocr_b_h: u16,
    pub icr_l: u16,
    pub icr_h: u16,
    pub timsk: u16,
    pub tifr: u16,
    pub gtccr: u16,
    pub v_capt: u8,
    pub v_ovf: u8,
    pub v_comp_a: u8,
    pub v_comp_b: u8,
    pub oc_a_gpio: usize,
    pub oc_b_gpio: usize,
    pub icp_gpio: u8,
    pub t_gpio: u8,
    /// PRR bit that stops this timer.
    pub prr_mask: u8,
}

const TOV: u8 = 0x01;
const OCFA: u8 = 0x02;
const OCFB: u8 = 0x04;
const ICF: u8 = 0x20;
const PRESCALE: [i64; 8] = [0, 1, 8, 64, 256, 1024, -1, -1];
const EV_TICK: u8 = 0;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Normal,
    Ctc,
    Fast,
    Pc,
    Pfc,
}

const MODE_KIND: [Kind; 16] = [
    Kind::Normal, Kind::Pc, Kind::Pc, Kind::Pc, Kind::Ctc, Kind::Fast, Kind::Fast, Kind::Fast,
    Kind::Pfc, Kind::Pfc, Kind::Pc, Kind::Pc, Kind::Ctc, Kind::Normal, Kind::Fast, Kind::Fast,
];
/// TOP source per mode: 0 fixed, 1 OCRA, 2 ICR.
const MODE_TOPSRC: [u8; 16] = [0, 0, 0, 0, 1, 0, 0, 0, 2, 1, 2, 1, 2, 0, 2, 1];
const MODE_FIXTOP: [u32; 16] = [0xffff, 0xff, 0x1ff, 0x3ff, 0, 0xff, 0x1ff, 0x3ff, 0, 0, 0, 0, 0, 0xffff, 0, 0];
const MODE_NAMES: [&str; 16] = [
    "Normal", "PWM, Phase Correct, 8-bit", "PWM, Phase Correct, 9-bit", "PWM, Phase Correct, 10-bit", "CTC (TOP=OCR0A)",
    "Fast PWM, 8-bit", "Fast PWM, 9-bit", "Fast PWM, 10-bit", "PWM, Phase & Freq Correct (TOP=ICR0)", "PWM, Phase & Freq Correct (TOP=OCR0A)",
    "PWM, Phase Correct (TOP=ICR0)", "PWM, Phase Correct (TOP=OCR0A)", "CTC (TOP=ICR0)", "Reserved", "Fast PWM (TOP=ICR0)", "Fast PWM (TOP=OCR0A)",
];

pub struct Timer16 {
    c: Timer16Config,
    tcnt: u32,
    dir: i32,
    ocr_a: u32,
    ocr_b: u32,
    ocr_a_buf: u32,
    ocr_b_buf: u32,
    icr: u32,
    temp: u8,
    tccr_a: u8,
    tccr_b: u8,
    tifr: u8,
    timsk: u8,
    tsm: bool,
    oc_a: u8,
    oc_b: u8,
    block_match: bool,
    /// Cycle up to which the counter state is valid.
    last_sync: u64,
    /// Prescaler origin cycle (ticks happen at ps_base + k*N).
    ps_base: u64,
    /// Clock divisor; 0 = stopped, -1 = external T0 pin.
    n: i64,
    sleep_halted: bool,
    power_reduced: bool,
    halt_start: u64,
    /// Set when the analog comparator output is routed to input capture (ACIC).
    pub ac_capture: bool,
}

impl Timer16 {
    pub fn new(c: Timer16Config) -> Self {
        Self {
            c,
            tcnt: 0,
            dir: 1,
            ocr_a: 0,
            ocr_b: 0,
            ocr_a_buf: 0,
            ocr_b_buf: 0,
            icr: 0,
            temp: 0,
            tccr_a: 0,
            tccr_b: 0,
            tifr: 0,
            timsk: 0,
            tsm: false,
            oc_a: 0,
            oc_b: 0,
            block_match: false,
            last_sync: 0,
            ps_base: 0,
            n: 0,
            sleep_halted: false,
            power_reduced: false,
            halt_start: 0,
            ac_capture: false,
        }
    }

    /// Owned registers with their SBI/CBI read-clear masks.
    pub fn registers(&self) -> Vec<(u16, u8)> {
        let c = &self.c;
        vec![
            (c.tccr_a, 0), (c.tccr_b, 0), (c.tccr_c, 0), (c.tcnt_l, 0), (c.tcnt_h, 0), (c.ocr_a_l, 0), (c.ocr_a_h, 0),
            (c.ocr_b_l, 0), (c.ocr_b_h, 0), (c.icr_l, 0), (c.icr_h, 0), (c.timsk, 0), (c.tifr, 0xff), (c.gtccr, 0),
        ]
    }

    // ------------------------------------------------------------------------------
    // Mode helpers
    // ------------------------------------------------------------------------------

    fn wgm(&self) -> usize {
        (((self.tccr_b >> 1) & 0x0c) | (self.tccr_a & 0x03)) as usize
    }

    fn kind(&self) -> Kind {
        MODE_KIND[self.wgm()]
    }

    fn top(&self) -> u32 {
        let w = self.wgm();
        match MODE_TOPSRC[w] {
            0 => MODE_FIXTOP[w],
            1 => self.ocr_a,
            _ => self.icr,
        }
    }

    fn buffered(&self) -> bool {
        matches!(self.kind(), Kind::Fast | Kind::Pc | Kind::Pfc)
    }

    fn running(&self) -> bool {
        self.n > 0 && !self.sleep_halted && !self.power_reduced && !self.tsm
    }

    // ------------------------------------------------------------------------------
    // Counting
    // ------------------------------------------------------------------------------

    /// Ticks until the next tick whose processing has observable effects (>= 1).
    fn distance(&self) -> u64 {
        let c = self.tcnt;
        let top = self.top();
        let (a, b) = (self.ocr_a, self.ocr_b);
        if self.dir > 0 || self.kind() < Kind::Pc {
            let lim = if c <= top { top } else { 0xffff };
            let mut d = lim - c + 1;
            if a >= c && a <= lim {
                d = d.min(a - c + 1);
            }
            if b >= c && b <= lim {
                d = d.min(b - c + 1);
            }
            return d as u64;
        }
        let mut d = c + 1;
        if a <= c {
            d = d.min(c - a + 1);
        }
        if b <= c {
            d = d.min(c - b + 1);
        }
        d as u64
    }

    fn advance_linear(&mut self, n: u64) {
        if n == 0 {
            return;
        }
        let delta = (n as i64 * self.dir as i64).rem_euclid(0x10000);
        self.tcnt = ((self.tcnt as i64 + delta) & 0xffff) as u32;
        self.block_match = false;
    }

    /// Processes one timer clock with full compare/TOP/BOTTOM logic.
    fn tick_once(&mut self, cycle: u64, cx: &mut Cx) {
        let old = self.tcnt;
        let kind = self.kind();
        let w = self.wgm();
        let top = self.top();
        let mut flags = 0u8;
        if !self.block_match {
            if old == self.ocr_a {
                flags |= OCFA;
                self.compare_output(0, old, top, kind, cycle, cx);
            }
            if old == self.ocr_b {
                flags |= OCFB;
                self.compare_output(1, old, top, kind, cycle, cx);
            }
        }
        self.block_match = false;
        let icr_top = MODE_TOPSRC[w] == 2;
        match kind {
            Kind::Normal => {
                if old == 0xffff {
                    flags |= TOV;
                }
                self.tcnt = (old + 1) & 0xffff;
            }
            Kind::Ctc => {
                if old == 0xffff {
                    flags |= TOV;
                }
                if old == top {
                    self.tcnt = 0;
                    if icr_top {
                        flags |= ICF;
                    }
                } else {
                    self.tcnt = (old + 1) & 0xffff;
                }
            }
            Kind::Fast => {
                if old == top {
                    self.tcnt = 0;
                    flags |= TOV;
                    if icr_top {
                        flags |= ICF;
                    }
                    self.ocr_a = self.ocr_a_buf;
                    self.ocr_b = self.ocr_b_buf;
                    self.bottom_output(cycle, cx);
                } else {
                    self.tcnt = (old + 1) & 0xffff;
                }
            }
            Kind::Pc | Kind::Pfc => {
                if self.dir > 0 {
                    if old == top {
                        self.dir = -1;
                        self.tcnt = if top == 0 { 0 } else { old - 1 };
                        if icr_top {
                            flags |= ICF;
                        }
                        if kind == Kind::Pc {
                            self.ocr_a = self.ocr_a_buf;
                            self.ocr_b = self.ocr_b_buf;
                        }
                    } else {
                        self.tcnt = (old + 1) & 0xffff;
                    }
                } else if old == 0 {
                    self.dir = 1;
                    self.tcnt = if top == 0 { 0 } else { 1 };
                    flags |= TOV;
                    if kind == Kind::Pfc {
                        self.ocr_a = self.ocr_a_buf;
                        self.ocr_b = self.ocr_b_buf;
                    }
                } else {
                    self.tcnt = old - 1;
                }
            }
        }
        if flags != 0 {
            self.set_flags(flags, cycle, cx);
        }
    }

    fn set_flags(&mut self, flags: u8, cycle: u64, cx: &mut Cx) {
        self.tifr |= flags;
        self.update_irq(cx);
        for (bit, t) in [(OCFA, Trigger::Tc0CompA), (OCFB, Trigger::Tc0CompB), (TOV, Trigger::Tc0Ovf), (ICF, Trigger::Tc0Capt)] {
            if flags & bit != 0 {
                cx.sys.trigger(t, 1, cycle);
            }
        }
    }

    fn sync(&mut self, now: u64, cx: &mut Cx) {
        if now <= self.last_sync {
            return;
        }
        if !self.running() {
            self.last_sync = now;
            return;
        }
        let n = self.n as u64;
        let mut k = (self.last_sync - self.ps_base) / n;
        let mut ticks = (now - self.ps_base) / n - k;
        while ticks > 0 {
            let d = self.distance();
            if d > ticks {
                self.advance_linear(ticks);
                break;
            }
            self.advance_linear(d - 1);
            k += d;
            ticks -= d;
            self.tick_once(self.ps_base + k * n, cx);
        }
        self.last_sync = now;
    }

    fn schedule(&mut self, cx: &mut Cx) {
        if !self.running() {
            cx.cancel(EV_TICK);
            return;
        }
        let n = self.n as u64;
        let k = (self.last_sync - self.ps_base) / n;
        let at = self.ps_base + (k + self.distance()) * n;
        cx.schedule(EV_TICK, at);
    }

    // ------------------------------------------------------------------------------
    // Output compare
    // ------------------------------------------------------------------------------

    fn com(&self, ch: usize) -> u8 {
        if ch == 0 { (self.tccr_a >> 6) & 3 } else { (self.tccr_a >> 4) & 3 }
    }

    /// Whether channel `ch` drives its pin in the current mode.
    fn output_enabled(&self, ch: usize) -> bool {
        match self.com(ch) {
            0 => false,
            1 => {
                matches!(self.kind(), Kind::Normal | Kind::Ctc) || (ch == 0 && matches!(self.wgm(), 9 | 11 | 15))
            }
            _ => true,
        }
    }

    fn compare_output(&mut self, ch: usize, old: u32, top: u32, kind: Kind, cycle: u64, cx: &mut Cx) {
        if !self.output_enabled(ch) {
            return;
        }
        let com = self.com(ch);
        let cur = if ch == 0 { self.oc_a } else { self.oc_b };
        let v = if com == 1 {
            cur ^ 1
        } else if matches!(kind, Kind::Normal | Kind::Ctc | Kind::Fast) {
            (com == 3) as u8
        } else {
            // Dual slope: a match at TOP behaves as down-counting, at BOTTOM as up-counting.
            let up = if old == top { false } else if old == 0 { true } else { self.dir > 0 };
            if (com == 2) == up { 0 } else { 1 }
        };
        self.set_oc(ch, v, cycle, cx);
    }

    fn bottom_output(&mut self, cycle: u64, cx: &mut Cx) {
        for ch in 0..2 {
            let com = self.com(ch);
            if com >= 2 {
                self.set_oc(ch, (com == 2) as u8, cycle, cx);
            }
        }
    }

    fn set_oc(&mut self, ch: usize, v: u8, cycle: u64, cx: &mut Cx) {
        if ch == 0 {
            self.oc_a = v;
        } else {
            self.oc_b = v;
        }
        self.apply_output(ch, cycle, cx);
    }

    fn apply_output(&mut self, ch: usize, cycle: u64, cx: &mut Cx) {
        let gpio = if ch == 0 { self.c.oc_a_gpio } else { self.c.oc_b_gpio };
        let en = self.output_enabled(ch) as u8;
        let val = if ch == 0 { self.oc_a } else { self.oc_b };
        let p = &mut cx.sys.pins[gpio];
        if p.ov_enable != en || p.ov_value != val {
            p.ov_enable = en;
            p.ov_value = val;
            cx.sys.update_pin(gpio, cycle);
        }
    }

    fn force_compare(&mut self, v: u8, cx: &mut Cx) {
        let k = self.kind();
        if !matches!(k, Kind::Normal | Kind::Ctc) {
            return;
        }
        let now = cx.now();
        self.sync(now, cx);
        let top = self.top();
        if v & 0x80 != 0 {
            self.compare_output(0, self.tcnt, top, k, now, cx);
        }
        if v & 0x40 != 0 {
            self.compare_output(1, self.tcnt, top, k, now, cx);
        }
    }

    // ------------------------------------------------------------------------------
    // Configuration
    // ------------------------------------------------------------------------------

    fn reconfigure(&mut self, cx: &mut Cx) {
        let new_n = PRESCALE[(self.tccr_b & 7) as usize];
        if new_n != self.n {
            let was_running = self.running();
            self.n = new_n;
            if !was_running {
                self.last_sync = cx.now();
            }
        }
        if !self.buffered() {
            self.ocr_a = self.ocr_a_buf;
            self.ocr_b = self.ocr_b_buf;
        }
        let now = cx.now();
        self.apply_output(0, now, cx);
        self.apply_output(1, now, cx);
        self.schedule(cx);
    }

    fn write_gtccr(&mut self, v: u8, cx: &mut Cx) {
        let now = cx.now();
        self.sync(now, cx);
        let tsm = v & 0x80 != 0;
        if v & 0x01 != 0 {
            self.ps_base = now;
        }
        if tsm != self.tsm {
            if tsm {
                self.halt_start = now;
            } else {
                self.last_sync = now;
                self.ps_base = now;
            }
            self.tsm = tsm;
        }
        self.schedule(cx);
    }

    // ------------------------------------------------------------------------------
    // Input capture
    // ------------------------------------------------------------------------------

    pub fn capture_edge(&mut self, level: u8, cycle: u64, cx: &mut Cx) {
        if MODE_TOPSRC[self.wgm()] == 2 {
            return; // ICR used as TOP
        }
        let rising = self.tccr_b & 0x40 != 0;
        if (level == 1) != rising {
            return;
        }
        self.sync(cycle, cx);
        self.icr = self.tcnt;
        self.set_flags(ICF, cycle, cx);
    }

    fn update_irq(&self, cx: &mut Cx) {
        let f = self.tifr & self.timsk;
        cx.cpu.set_irq(self.c.v_ovf, f & TOV != 0);
        cx.cpu.set_irq(self.c.v_comp_a, f & OCFA != 0);
        cx.cpu.set_irq(self.c.v_comp_b, f & OCFB != 0);
        cx.cpu.set_irq(self.c.v_capt, f & ICF != 0);
    }

    fn read16(&self, addr: u16) -> Option<u32> {
        let c = &self.c;
        if addr == c.tcnt_l || addr == c.tcnt_h {
            Some(self.tcnt)
        } else if addr == c.ocr_a_l || addr == c.ocr_a_h {
            Some(self.ocr_a_buf)
        } else if addr == c.ocr_b_l || addr == c.ocr_b_h {
            Some(self.ocr_b_buf)
        } else if addr == c.icr_l || addr == c.icr_h {
            Some(self.icr)
        } else {
            None
        }
    }

    fn is_high(&self, addr: u16) -> bool {
        let c = &self.c;
        addr == c.tcnt_h || addr == c.ocr_a_h || addr == c.ocr_b_h || addr == c.icr_h
    }
}

impl Peripheral for Timer16 {
    fn name(&self) -> &str {
        self.c.name
    }

    fn read(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        let now = cx.now();
        if self.is_high(addr) {
            return self.temp;
        }
        if let Some(v) = self.read16(addr) {
            self.sync(now, cx);
            let v = self.read16(addr).unwrap_or(v);
            self.temp = (v >> 8) as u8;
            return v as u8;
        }
        self.peek(addr, cx)
    }

    fn peek(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        let now = cx.now();
        self.sync(now, cx);
        let c = &self.c;
        if let Some(v) = self.read16(addr) {
            return if self.is_high(addr) { (v >> 8) as u8 } else { v as u8 };
        }
        if addr == c.tifr {
            self.tifr
        } else if addr == c.timsk {
            self.timsk
        } else if addr == c.tccr_a {
            self.tccr_a
        } else if addr == c.tccr_b {
            self.tccr_b
        } else if addr == c.gtccr {
            if self.tsm { 0x81 } else { 0 }
        } else {
            0
        }
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let now = cx.now();
        let c = &self.c;
        if self.is_high(addr) {
            self.temp = v;
            return;
        }
        if self.read16(addr).is_some() {
            self.sync(now, cx);
            let val = ((self.temp as u32) << 8) | v as u32;
            let c = &self.c;
            if addr == c.tcnt_l {
                self.tcnt = val;
                self.block_match = true;
            } else if addr == c.ocr_a_l {
                self.ocr_a_buf = val;
                if !self.buffered() {
                    self.ocr_a = val;
                }
            } else if addr == c.ocr_b_l {
                self.ocr_b_buf = val;
                if !self.buffered() {
                    self.ocr_b = val;
                }
            } else {
                self.icr = val;
            }
            self.schedule(cx);
            return;
        }
        if addr == c.tifr {
            self.sync(now, cx);
            self.tifr &= !v;
            self.update_irq(cx);
        } else if addr == c.timsk {
            self.timsk = v & 0x27;
            self.update_irq(cx);
        } else if addr == c.tccr_a {
            self.sync(now, cx);
            self.tccr_a = v & 0xf3;
            self.reconfigure(cx);
        } else if addr == c.tccr_b {
            self.sync(now, cx);
            self.tccr_b = v & 0xdf;
            self.reconfigure(cx);
        } else if addr == c.tccr_c {
            self.force_compare(v, cx);
        } else if addr == c.gtccr {
            self.write_gtccr(v, cx);
        }
    }

    fn on_event(&mut self, _tag: u8, cycle: u64, cx: &mut Cx) {
        self.sync(cycle, cx);
        self.schedule(cx);
    }

    fn ack(&mut self, vector: u8, cx: &mut Cx) {
        let now = cx.now();
        self.sync(now, cx);
        let c = &self.c;
        let flag = if vector == c.v_ovf {
            TOV
        } else if vector == c.v_comp_a {
            OCFA
        } else if vector == c.v_comp_b {
            OCFB
        } else {
            ICF
        };
        self.tifr &= !flag;
        self.update_irq(cx);
    }

    fn on_pin(&mut self, pin: u8, level: u8, cycle: u64, cx: &mut Cx) {
        if pin == self.c.icp_gpio && !self.ac_capture {
            self.capture_edge(level, cycle, cx);
        }
        if pin == self.c.t_gpio && self.n == -1 && !self.sleep_halted && !self.power_reduced {
            let cs = self.tccr_b & 7;
            if (cs == 6 && level == 0) || (cs == 7 && level == 1) {
                self.tick_once(cycle, cx);
                self.last_sync = cycle.max(self.last_sync);
            }
        }
    }

    fn on_trigger(&mut self, trigger: Trigger, value: u8, cycle: u64, cx: &mut Cx) {
        if trigger == Trigger::AcOutput && self.ac_capture {
            self.capture_edge(value, cycle, cx);
        }
    }

    fn on_power_reduction(&mut self, prr: u8, cx: &mut Cx) {
        let on = prr & self.c.prr_mask != 0;
        if on == self.power_reduced {
            return;
        }
        let now = cx.now();
        self.sync(now, cx);
        self.power_reduced = on;
        self.last_sync = now;
        self.schedule(cx);
    }

    fn on_sleep(&mut self, mode: u8, cx: &mut Cx) {
        if mode == 0 {
            return; // idle: clk_IO keeps running
        }
        let now = cx.now();
        self.sync(now, cx);
        self.sleep_halted = true;
        self.halt_start = now;
        self.schedule(cx);
    }

    fn on_wake(&mut self, cx: &mut Cx) {
        if !self.sleep_halted {
            return;
        }
        let now = cx.now();
        self.sleep_halted = false;
        // The prescaler is halted together with the counter: keep the tick phase.
        self.ps_base += now - self.halt_start;
        self.last_sync = now;
        self.schedule(cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        cx.cancel(EV_TICK);
        *self = Timer16::new(self.c.clone());
        self.last_sync = cx.now();
        self.ps_base = cx.now();
        let now = cx.now();
        self.apply_output(0, now, cx);
        self.apply_output(1, now, cx);
        self.update_irq(cx);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        let now = cx.now();
        self.sync(now, cx);
        let cs = self.tccr_b & 7;
        let clock = match cs {
            0 => "Stopped".to_string(),
            6 => "T0 pin (falling)".into(),
            7 => "T0 pin (rising)".into(),
            _ => format!("clk/{}", PRESCALE[cs as usize]),
        };
        let state = if self.power_reduced {
            "Power reduced"
        } else if self.sleep_halted {
            "Halted (sleep)"
        } else if self.tsm {
            "Halted (TSM)"
        } else if self.running() || self.n == -1 {
            "Running"
        } else {
            "Stopped"
        };
        vec![
            ("Mode".into(), format!("{}: {}", self.wgm(), MODE_NAMES[self.wgm()])),
            ("Clock".into(), clock),
            ("State".into(), state.into()),
            ("TCNT0".into(), self.tcnt.to_string()),
            ("TOP".into(), self.top().to_string()),
            ("Direction".into(), if self.dir > 0 { "Up" } else { "Down" }.into()),
            ("OCR0A (active)".into(), self.ocr_a.to_string()),
            ("OCR0B (active)".into(), self.ocr_b.to_string()),
            ("OC0A".into(), self.oc_a.to_string()),
            ("OC0B".into(), self.oc_b.to_string()),
            ("TEMP".into(), format!("0x{:02X}", self.temp)),
        ]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
