//! AVR Timer/Counter: the 16-bit timers (ATtiny10 Timer0, ATmega Timer1: 16 waveform generation
//! modes, input capture, TEMP register) and the 8-bit ones (ATmega Timer0/Timer2, ATtiny85
//! Timer0: 8 modes). Normal, CTC, fast PWM, phase correct and phase & frequency correct PWM,
//! two output compare units (three on the 16-bit timers of the ATmega640/1280/2560) with pin
//! outputs, external clock input.
//!
//! Sources: Atmel-8127H ATtiny4/5/9/10 section 12, Atmel ATmega48A/PA/88A/PA/168A/PA/328/P
//! (DS40002061B) sections 15-18, Atmel-2586Q ATtiny25/45/85 section 11, Atmel-2486AA / 2466T /
//! 2503Q (ATmega8/16/32: single TCCRn register layout, one compare unit, FOC1x in TCCR1A),
//! Atmel-2549Q ATmega640/1280/2560 section 17 (third compare unit OCRnC / OCFnC / OCnC / FOCnC).
//!
//! Event driven: the counter is advanced lazily ("synced") to the current cycle whenever
//! software touches a register, and one scheduler event is armed for the next timer tick where
//! something observable happens (compare match, TOP, BOTTOM, MAX). Between those ticks the count
//! is linear, so syncing costs O(1) per event regardless of how many ticks elapsed.
//! Interrupt flags live in the (possibly shared) TIFR register owned by `IrqFlags`.

use crate::avr::machine::{Cx, Peripheral, Trigger};
use crate::avr::peripherals::irqflags::update_irqs;

/// Clock select divisors for CS2:0 = 0..7 (0 = stopped).
pub type ClockSelect = [Clk; 8];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clk {
    Stop,
    Div(u32),
    /// External pin, falling edge.
    ExtFall,
    /// External pin, rising edge.
    ExtRise,
}

/// Timer0/Timer1 of the classic AVRs and the ATtiny10 Timer0.
pub const CS_SYNC: ClockSelect = [Clk::Stop, Clk::Div(1), Clk::Div(8), Clk::Div(64), Clk::Div(256), Clk::Div(1024), Clk::ExtFall, Clk::ExtRise];
/// ATmega Timer2 (no external clock pin, extra /32 and /128 taps).
pub const CS_TIMER2: ClockSelect = [Clk::Stop, Clk::Div(1), Clk::Div(8), Clk::Div(32), Clk::Div(64), Clk::Div(128), Clk::Div(256), Clk::Div(1024)];

/// Control register layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerLayout {
    /// TCCRnA / TCCRnB (+ TCCRnC) as on the ATmega48..328 and the ATtiny parts.
    Split,
    /// One TCCRn register at `tccr_a` (ATmega8/16/32 Timer0/2): bit 7 FOCn strobe (reads 0),
    /// bit 6 WGMn0, bits 5:4 COMn, bit 3 WGMn1, bits 2:0 CS. Without `wgm` (ATmega8 Timer0) only
    /// the clock select is writable and there is no compare unit.
    Single { wgm: bool },
}

/// FOCnA / FOCnB masks of the classic layout (TCCRnB bits 7:6 / TCCRnC).
pub const FOC_STD: (u8, u8) = (0x80, 0x40);

/// Flag / enable bit masks in TIFRn / TIMSKn.
#[derive(Clone, Copy)]
pub struct TimerBits {
    pub tov: u8,
    pub ocfa: u8,
    pub ocfb: u8,
    /// 0 when the timer has no input capture unit.
    pub icf: u8,
}

/// Optional third output compare unit C of the 16-bit timers of the ATmega640/1280/2560
/// (OCRnC low byte, OCFnC / OCIEnC, COMnC in TCCRnA bits 3:2, FOCnC in TCCRnC bit 5, pin OCnC).
#[derive(Clone, Copy)]
pub struct CompareC {
    pub ocr: u16,
    /// OCFnC / OCIEnC bit in TIFRn / TIMSKn.
    pub flag: u8,
    pub vector: u8,
    pub gpio: Option<usize>,
    /// FOCnC strobe mask within `TimerConfig::foc_reg`.
    pub foc: u8,
}

#[derive(Clone)]
pub struct TimerConfig {
    pub name: &'static str,
    /// Timer number (registers names and trigger ids).
    pub id: u8,
    /// 16-bit timer (TEMP register, 16 modes, input capture).
    pub wide: bool,
    pub layout: TimerLayout,
    pub tccr_a: u16,
    /// Same address as `tccr_a` for `TimerLayout::Single`.
    pub tccr_b: u16,
    /// Register holding FOCnA / FOCnB (TCCRnC on 16-bit timers, TCCRnB on 8-bit ones, TCCRnA on
    /// the ATmega8/16/32 Timer1 and for `TimerLayout::Single`).
    pub foc_reg: u16,
    /// FOCnA / FOCnB masks within `foc_reg` (0 = no such channel).
    pub foc_bits: (u8, u8),
    pub tcnt: u16,
    /// Output compare registers (low byte); None when the compare unit does not exist.
    pub ocr_a: Option<u16>,
    pub ocr_b: Option<u16>,
    /// Input capture register (low byte address), 16-bit timers only.
    pub icr: Option<u16>,
    pub tifr: u16,
    pub timsk: u16,
    /// Flag bits of absent compare units must be 0.
    pub bits: TimerBits,
    pub v_ovf: u8,
    pub v_comp_a: Option<u8>,
    pub v_comp_b: Option<u8>,
    pub v_capt: Option<u8>,
    pub oc_a_gpio: Option<usize>,
    pub oc_b_gpio: Option<usize>,
    /// Third compare unit (None on every timer except the 2560-style 16-bit ones).
    pub c_unit: Option<CompareC>,
    pub icp_gpio: Option<u8>,
    pub t_gpio: Option<u8>,
    pub clock: ClockSelect,
    /// Prescaler group reset by GTCCR (Trigger::PrescalerReset mask).
    pub prescaler_group: u8,
    /// PRR bit that stops this timer.
    pub prr_mask: u16,
    /// Canonical sleep modes (bit per `SleepKind`) in which the timer keeps counting. Idle is
    /// always included.
    pub sleep_run: u8,
}

const EV_TICK: u8 = 0;
/// OCR value of an absent compare unit: never matches.
const NO_OCR: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Normal,
    Ctc,
    Fast,
    Pc,
    Pfc,
}

struct Modes {
    kind: &'static [Kind],
    /// TOP source per mode: 0 fixed, 1 OCRA, 2 ICR.
    topsrc: &'static [u8],
    fixtop: &'static [u32],
    names: &'static [&'static str],
    /// Modes in which COMnA = 01 toggles OCnA on compare match.
    toggle_a: &'static [usize],
}

const MODES16: Modes = Modes {
    kind: &[
        Kind::Normal, Kind::Pc, Kind::Pc, Kind::Pc, Kind::Ctc, Kind::Fast, Kind::Fast, Kind::Fast,
        Kind::Pfc, Kind::Pfc, Kind::Pc, Kind::Pc, Kind::Ctc, Kind::Normal, Kind::Fast, Kind::Fast,
    ],
    topsrc: &[0, 0, 0, 0, 1, 0, 0, 0, 2, 1, 2, 1, 2, 0, 2, 1],
    fixtop: &[0xffff, 0xff, 0x1ff, 0x3ff, 0, 0xff, 0x1ff, 0x3ff, 0, 0, 0, 0, 0, 0xffff, 0, 0],
    names: &[
        "Normal", "PWM, Phase Correct, 8-bit", "PWM, Phase Correct, 9-bit", "PWM, Phase Correct, 10-bit", "CTC (TOP=OCRnA)",
        "Fast PWM, 8-bit", "Fast PWM, 9-bit", "Fast PWM, 10-bit", "PWM, Phase & Freq Correct (TOP=ICRn)", "PWM, Phase & Freq Correct (TOP=OCRnA)",
        "PWM, Phase Correct (TOP=ICRn)", "PWM, Phase Correct (TOP=OCRnA)", "CTC (TOP=ICRn)", "Reserved", "Fast PWM (TOP=ICRn)", "Fast PWM (TOP=OCRnA)",
    ],
    toggle_a: &[9, 11, 15],
};

const MODES8: Modes = Modes {
    kind: &[Kind::Normal, Kind::Pc, Kind::Ctc, Kind::Fast, Kind::Normal, Kind::Pc, Kind::Normal, Kind::Fast],
    topsrc: &[0, 0, 1, 0, 0, 1, 0, 1],
    fixtop: &[0xff, 0xff, 0, 0xff, 0xff, 0, 0xff, 0],
    names: &["Normal", "PWM, Phase Correct", "CTC (TOP=OCRnA)", "Fast PWM", "Reserved", "PWM, Phase Correct (TOP=OCRnA)", "Reserved", "Fast PWM (TOP=OCRnA)"],
    toggle_a: &[5, 7],
};

pub struct Timer {
    c: TimerConfig,
    m: &'static Modes,
    max: u32,
    irq_map: [(u8, u8); 5],
    tcnt: u32,
    dir: i32,
    /// Active and buffered compare values of channels A, B, C.
    ocr: [u32; 3],
    ocr_buf: [u32; 3],
    /// Compare channels this timer has (2 or 3; the loops over channels stop here).
    nch: usize,
    icr: u32,
    temp: u8,
    tccr_a: u8,
    tccr_b: u8,
    tsm: bool,
    oc: [u8; 3],
    block_match: bool,
    /// Cycle up to which the counter state is valid.
    last_sync: u64,
    /// Prescaler origin cycle (ticks happen at ps_base + k*N).
    ps_base: u64,
    clk: Clk,
    sleep_halted: bool,
    power_reduced: bool,
    halt_start: u64,
    /// The analog comparator output is routed to input capture (ACIC).
    ac_capture: bool,
}

impl Timer {
    pub fn new(c: TimerConfig) -> Self {
        let m = if c.wide { &MODES16 } else { &MODES8 };
        let b = c.bits;
        let (fc, vc) = c.c_unit.map_or((0, 0), |u| (u.flag, u.vector));
        let irq_map = [(b.tov, c.v_ovf), (b.ocfa, c.v_comp_a.unwrap_or(0)), (b.ocfb, c.v_comp_b.unwrap_or(0)), (b.icf, c.v_capt.unwrap_or(0)), (fc, vc)];
        let absent = |o: bool| if o { 0 } else { NO_OCR };
        let nch = if c.c_unit.is_some() { 3 } else { 2 };
        let o = [absent(c.ocr_a.is_some()), absent(c.ocr_b.is_some()), absent(c.c_unit.is_some())];
        Self {
            max: if c.wide { 0xffff } else { 0xff },
            m,
            irq_map,
            c,
            tcnt: 0,
            dir: 1,
            ocr: o,
            ocr_buf: o,
            nch,
            icr: 0,
            temp: 0,
            tccr_a: 0,
            tccr_b: 0,
            tsm: false,
            oc: [0; 3],
            block_match: false,
            last_sync: 0,
            ps_base: 0,
            clk: Clk::Stop,
            sleep_halted: false,
            power_reduced: false,
            halt_start: 0,
            ac_capture: false,
        }
    }

    /// Owned registers (TIFR/TIMSK belong to the flag register owner).
    pub fn registers(&self) -> Vec<(u16, u8)> {
        let c = &self.c;
        let mut v = vec![(c.tccr_a, 0), (c.tcnt, 0)];
        if c.tccr_b != c.tccr_a {
            v.push((c.tccr_b, 0));
        }
        for o in [c.ocr_a, c.ocr_b, c.c_unit.map(|u| u.ocr)].into_iter().flatten() {
            v.push((o, 0));
            if c.wide {
                v.push((o + 1, 0));
            }
        }
        if c.wide {
            v.push((c.tcnt + 1, 0));
            if let Some(i) = c.icr {
                v.extend([(i, 0), (i + 1, 0)]);
            }
        }
        if c.foc_reg != c.tccr_a && c.foc_reg != c.tccr_b {
            v.push((c.foc_reg, 0));
        }
        v
    }

    /// The (flag bit, vector) pairs of this timer, for its `IrqFlags` owner.
    pub fn irq_map(&self) -> Vec<(u8, u8)> {
        self.irq_map.iter().copied().filter(|e| e.0 != 0).collect()
    }

    // ------------------------------------------------------------------------------
    // Mode helpers
    // ------------------------------------------------------------------------------

    fn wgm(&self) -> usize {
        let hi = if self.c.wide { (self.tccr_b >> 1) & 0x0c } else { (self.tccr_b >> 1) & 0x04 };
        (hi | (self.tccr_a & 0x03)) as usize
    }

    fn kind(&self) -> Kind {
        self.m.kind[self.wgm()]
    }

    fn top(&self) -> u32 {
        let w = self.wgm();
        match self.m.topsrc[w] {
            0 => self.m.fixtop[w],
            1 => self.ocr[0],
            _ => self.icr,
        }
    }

    fn buffered(&self) -> bool {
        matches!(self.kind(), Kind::Fast | Kind::Pc | Kind::Pfc)
    }

    fn div(&self) -> u64 {
        match self.clk {
            Clk::Div(n) => n as u64,
            _ => 0,
        }
    }

    fn running(&self) -> bool {
        self.div() > 0 && !self.sleep_halted && !self.power_reduced && !self.tsm
    }

    fn external(&self) -> bool {
        matches!(self.clk, Clk::ExtFall | Clk::ExtRise)
    }

    fn set_flags(&mut self, flags: u8, cycle: u64, cx: &mut Cx) {
        let b = self.c.bits;
        let mut data = 0u8;
        let fc = self.c.c_unit.map_or(0, |u| u.flag);
        for (bit, mine) in [(TOV, b.tov), (OCFA, b.ocfa), (OCFB, b.ocfb), (ICF, b.icf), (OCFC, fc)] {
            if flags & bit != 0 {
                data |= mine;
            }
        }
        cx.cpu.data[self.c.tifr as usize] |= data;
        update_irqs(cx, self.c.tifr, self.c.timsk, &self.irq_map);
        let id = self.c.id;
        for (bit, t) in [(OCFA, Trigger::TimerCompA(id)), (OCFB, Trigger::TimerCompB(id)), (TOV, Trigger::TimerOvf(id)), (ICF, Trigger::TimerCapt(id))] {
            if flags & bit != 0 {
                cx.sys.trigger(t, 1, cycle);
            }
        }
    }

    // ------------------------------------------------------------------------------
    // Counting
    // ------------------------------------------------------------------------------

    /// Ticks until the next tick whose processing has observable effects (>= 1).
    fn distance(&self) -> u64 {
        let c = self.tcnt;
        let top = self.top();
        if self.dir > 0 || self.kind() < Kind::Pc {
            let lim = if c <= top { top } else { self.max };
            let mut d = lim - c + 1;
            for &o in &self.ocr {
                if o >= c && o <= lim {
                    d = d.min(o - c + 1);
                }
            }
            return d as u64;
        }
        let mut d = c + 1;
        for &o in &self.ocr {
            if o <= c {
                d = d.min(c - o + 1);
            }
        }
        d as u64
    }

    fn advance_linear(&mut self, n: u64) {
        if n == 0 {
            return;
        }
        let span = self.max as i64 + 1;
        let delta = (n as i64 * self.dir as i64).rem_euclid(span);
        self.tcnt = ((self.tcnt as i64 + delta) & self.max as i64) as u32;
        self.block_match = false;
    }

    /// Processes one timer clock with full compare/TOP/BOTTOM logic.
    fn tick_once(&mut self, cycle: u64, cx: &mut Cx) {
        let old = self.tcnt;
        let kind = self.kind();
        let top = self.top();
        let max = self.max;
        let mut flags = 0u8;
        if !self.block_match {
            for (ch, flag) in CH_FLAGS.into_iter().enumerate().take(self.nch) {
                if old == self.ocr[ch] {
                    flags |= flag;
                    self.compare_output(ch, old, top, kind, cycle, cx);
                }
            }
        }
        self.block_match = false;
        let icr_top = self.m.topsrc[self.wgm()] == 2;
        match kind {
            Kind::Normal => {
                if old == max {
                    flags |= TOV;
                }
                self.tcnt = (old + 1) & max;
            }
            Kind::Ctc => {
                if old == max {
                    flags |= TOV;
                }
                if old == top {
                    self.tcnt = 0;
                    if icr_top {
                        flags |= ICF;
                    }
                } else {
                    self.tcnt = (old + 1) & max;
                }
            }
            Kind::Fast => {
                if old == top {
                    self.tcnt = 0;
                    flags |= TOV;
                    if icr_top {
                        flags |= ICF;
                    }
                    self.ocr = self.ocr_buf;
                    self.bottom_output(cycle, cx);
                } else {
                    self.tcnt = (old + 1) & max;
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
                            self.ocr = self.ocr_buf;
                        }
                    } else {
                        self.tcnt = (old + 1) & max;
                    }
                } else if old == 0 {
                    self.dir = 1;
                    self.tcnt = if top == 0 { 0 } else { 1 };
                    flags |= TOV;
                    if kind == Kind::Pfc {
                        self.ocr = self.ocr_buf;
                    }
                } else {
                    self.tcnt = old - 1;
                }
            }
        }
        if flags & ICF != 0 && self.c.bits.icf == 0 {
            flags &= !ICF;
        }
        if flags != 0 {
            self.set_flags(flags, cycle, cx);
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
        let n = self.div();
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
        let n = self.div();
        let k = (self.last_sync - self.ps_base) / n;
        let at = self.ps_base + (k + self.distance()) * n;
        cx.schedule(EV_TICK, at);
    }

    // ------------------------------------------------------------------------------
    // Output compare
    // ------------------------------------------------------------------------------

    fn com(&self, ch: usize) -> u8 {
        (self.tccr_a >> (6 - 2 * ch)) & 3
    }

    /// Whether channel `ch` drives its pin in the current mode.
    fn output_enabled(&self, ch: usize) -> bool {
        match self.com(ch) {
            0 => false,
            1 => matches!(self.kind(), Kind::Normal | Kind::Ctc) || (ch == 0 && self.m.toggle_a.contains(&self.wgm())),
            _ => true,
        }
    }

    fn compare_output(&mut self, ch: usize, old: u32, top: u32, kind: Kind, cycle: u64, cx: &mut Cx) {
        if !self.output_enabled(ch) {
            return;
        }
        let com = self.com(ch);
        let cur = self.oc[ch];
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
        for ch in 0..self.nch {
            let com = self.com(ch);
            if com >= 2 {
                self.set_oc(ch, (com == 2) as u8, cycle, cx);
            }
        }
    }

    fn set_oc(&mut self, ch: usize, v: u8, cycle: u64, cx: &mut Cx) {
        self.oc[ch] = v;
        self.apply_output(ch, cycle, cx);
    }

    fn apply_output(&mut self, ch: usize, cycle: u64, cx: &mut Cx) {
        let Some(gpio) = (match ch {
            0 => self.c.oc_a_gpio,
            1 => self.c.oc_b_gpio,
            _ => self.c.c_unit.and_then(|u| u.gpio),
        }) else {
            return;
        };
        let en = self.output_enabled(ch) as u8;
        let val = self.oc[ch];
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
        let (fa, fb) = self.c.foc_bits;
        if v & fa != 0 {
            self.compare_output(0, self.tcnt, top, k, now, cx);
        }
        if v & fb != 0 {
            self.compare_output(1, self.tcnt, top, k, now, cx);
        }
        if let Some(u) = self.c.c_unit {
            if v & u.foc != 0 {
                self.compare_output(2, self.tcnt, top, k, now, cx);
            }
        }
    }

    // ------------------------------------------------------------------------------
    // Configuration
    // ------------------------------------------------------------------------------

    fn reconfigure(&mut self, cx: &mut Cx) {
        let new_clk = self.c.clock[(self.tccr_b & 7) as usize];
        if new_clk != self.clk {
            let was_running = self.running();
            self.clk = new_clk;
            if !was_running {
                self.last_sync = cx.now();
            }
        }
        if !self.buffered() {
            self.ocr = self.ocr_buf;
        }
        let now = cx.now();
        for ch in 0..self.nch {
            self.apply_output(ch, now, cx);
        }
        self.schedule(cx);
    }

    fn set_tsm(&mut self, tsm: bool, cx: &mut Cx) {
        if tsm == self.tsm {
            return;
        }
        let now = cx.now();
        self.sync(now, cx);
        if tsm {
            self.halt_start = now;
        } else {
            self.last_sync = now;
            self.ps_base = now;
        }
        self.tsm = tsm;
        self.schedule(cx);
    }

    // ------------------------------------------------------------------------------
    // Input capture
    // ------------------------------------------------------------------------------

    fn capture_edge(&mut self, level: u8, cycle: u64, cx: &mut Cx) {
        if self.c.icr.is_none() || self.m.topsrc[self.wgm()] == 2 {
            return; // no capture unit / ICR used as TOP
        }
        let rising = self.tccr_b & 0x40 != 0;
        if (level == 1) != rising {
            return;
        }
        self.sync(cycle, cx);
        self.icr = self.tcnt;
        self.set_flags(ICF, cycle, cx);
    }

    /// 16-bit value behind a register address and whether it is the high byte.
    fn reg16(&self, addr: u16) -> Option<(u32, bool)> {
        let c = &self.c;
        let pair = |lo: Option<u16>| lo.and_then(|lo| if addr == lo { Some(false) } else if c.wide && addr == lo + 1 { Some(true) } else { None });
        if let Some(h) = pair(Some(c.tcnt)) {
            Some((self.tcnt, h))
        } else if let Some(h) = pair(c.ocr_a) {
            Some((self.ocr_buf[0], h))
        } else if let Some(h) = pair(c.ocr_b) {
            Some((self.ocr_buf[1], h))
        } else if let Some(h) = pair(c.c_unit.map(|u| u.ocr)) {
            Some((self.ocr_buf[2], h))
        } else {
            pair(c.icr).map(|h| (self.icr, h))
        }
    }
}

// Internal flag bits (mapped to the device's TIFR layout in `set_flags`).
const TOV: u8 = 0x01;
const OCFA: u8 = 0x02;
const OCFB: u8 = 0x04;
const OCFC: u8 = 0x08;
const ICF: u8 = 0x20;
/// Internal flag bit per compare channel.
const CH_FLAGS: [u8; 3] = [OCFA, OCFB, OCFC];

impl Peripheral for Timer {
    fn name(&self) -> &str {
        self.c.name
    }

    fn read(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        let now = cx.now();
        if let Some((_, high)) = self.reg16(addr) {
            if high {
                return self.temp;
            }
            self.sync(now, cx);
            let (v, _) = self.reg16(addr).unwrap_or((0, false));
            if self.c.wide {
                self.temp = (v >> 8) as u8;
            }
            return v as u8;
        }
        self.peek(addr, cx)
    }

    fn peek(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        let now = cx.now();
        self.sync(now, cx);
        if let Some((v, high)) = self.reg16(addr) {
            return if high { (v >> 8) as u8 } else { v as u8 };
        }
        if addr == self.c.tccr_a {
            match self.c.layout {
                TimerLayout::Split => self.tccr_a,
                TimerLayout::Single { .. } => {
                    // Rebuild the single register: COM (bits 5:4), WGM1 (3), WGM0 (6), CS (2:0).
                    ((self.tccr_a >> 6) & 3) << 4 | (self.tccr_a & 1) << 6 | ((self.tccr_a >> 1) & 1) << 3 | (self.tccr_b & 7)
                }
            }
        } else if addr == self.c.tccr_b {
            self.tccr_b
        } else {
            0 // FOC strobes read as zero
        }
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let now = cx.now();
        if let Some((_, high)) = self.reg16(addr) {
            if high {
                self.temp = v;
                return;
            }
            self.sync(now, cx);
            let val = if self.c.wide { ((self.temp as u32) << 8) | v as u32 } else { v as u32 };
            let c = &self.c;
            if addr == c.tcnt {
                self.tcnt = val;
                self.block_match = true;
            } else if let Some(ch) = [c.ocr_a, c.ocr_b, c.c_unit.map(|u| u.ocr)].iter().position(|&o| o == Some(addr)) {
                self.ocr_buf[ch] = val;
                if !self.buffered() {
                    self.ocr[ch] = val;
                }
            } else {
                self.icr = val;
            }
            self.schedule(cx);
            return;
        }
        if let TimerLayout::Single { wgm } = self.c.layout {
            if addr == self.c.tccr_a {
                self.sync(now, cx);
                self.tccr_b = v & 7;
                self.tccr_a = if wgm { ((v >> 4) & 3) << 6 | (v >> 6) & 1 | ((v >> 3) & 1) << 1 } else { 0 };
                self.reconfigure(cx);
                if wgm {
                    self.force_compare(v, cx);
                }
                return;
            }
        }
        if addr == self.c.tccr_a {
            self.sync(now, cx);
            self.tccr_a = v & if self.nch == 3 { 0xff } else { 0xf3 };
            self.reconfigure(cx);
            if self.c.foc_reg == addr {
                self.force_compare(v, cx);
            }
            return;
        }
        if addr == self.c.tccr_b {
            self.sync(now, cx);
            self.tccr_b = v & if self.c.wide { 0xdf } else { 0x0f };
            self.reconfigure(cx);
        }
        if addr == self.c.foc_reg {
            self.force_compare(v, cx);
        }
    }

    fn on_event(&mut self, _tag: u8, cycle: u64, cx: &mut Cx) {
        self.sync(cycle, cx);
        self.schedule(cx);
    }

    fn on_pin(&mut self, pin: u8, level: u8, cycle: u64, cx: &mut Cx) {
        if Some(pin) == self.c.icp_gpio && !self.ac_capture {
            self.capture_edge(level, cycle, cx);
        }
        let edge = (self.clk == Clk::ExtFall && level == 0) || (self.clk == Clk::ExtRise && level == 1);
        if Some(pin) == self.c.t_gpio && edge && !self.sleep_halted && !self.power_reduced && !self.tsm {
            self.tick_once(cycle, cx);
            self.last_sync = cycle.max(self.last_sync);
        }
    }

    fn on_trigger(&mut self, trigger: Trigger, value: u8, cycle: u64, cx: &mut Cx) {
        match trigger {
            Trigger::AcOutput if self.ac_capture => self.capture_edge(value, cycle, cx),
            Trigger::AcCapture if self.c.icr.is_some() => self.ac_capture = value != 0,
            Trigger::TimerSync => self.set_tsm(value != 0, cx),
            Trigger::PrescalerReset if value & self.c.prescaler_group != 0 => {
                let now = cx.now();
                self.sync(now, cx);
                self.ps_base = now;
                self.schedule(cx);
            }
            _ => {}
        }
    }

    fn on_power_reduction(&mut self, prr: u16, cx: &mut Cx) {
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
        if mode == 0 || self.c.sleep_run & (1 << mode) != 0 {
            return; // clk_IO (or the asynchronous clock) keeps running
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
        *self = Timer::new(self.c.clone());
        self.last_sync = cx.now();
        self.ps_base = cx.now();
        let now = cx.now();
        for ch in 0..self.nch {
            self.apply_output(ch, now, cx);
        }
        update_irqs(cx, self.c.tifr, self.c.timsk, &self.irq_map);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        let now = cx.now();
        self.sync(now, cx);
        let n = self.c.id;
        let clock = match self.clk {
            Clk::Stop => "Stopped".to_string(),
            Clk::ExtFall => format!("T{n} pin (falling)"),
            Clk::ExtRise => format!("T{n} pin (rising)"),
            Clk::Div(d) => format!("clk/{d}"),
        };
        let state = if self.power_reduced {
            "Power reduced"
        } else if self.sleep_halted {
            "Halted (sleep)"
        } else if self.tsm {
            "Halted (TSM)"
        } else if self.running() || self.external() {
            "Running"
        } else {
            "Stopped"
        };
        let mut v = vec![
            ("Mode".into(), format!("{}: {}", self.wgm(), self.m.names[self.wgm()].replace("OCRnA", &format!("OCR{n}A")).replace("ICRn", &format!("ICR{n}")))),
            ("Clock".into(), clock),
            ("State".into(), state.into()),
            (format!("TCNT{n}"), self.tcnt.to_string()),
            ("TOP".into(), self.top().to_string()),
            ("Direction".into(), if self.dir > 0 { "Up" } else { "Down" }.into()),
        ];
        // Single-channel timers show "OCn" instead of "OCnA".
        let one = self.c.ocr_b.is_none();
        if self.c.ocr_a.is_some() {
            v.push((format!("OCR{n}{} (active)", if one { "" } else { "A" }), self.ocr[0].to_string()));
            v.push((format!("OC{n}{}", if one { "" } else { "A" }), self.oc[0].to_string()));
        }
        if self.c.ocr_b.is_some() {
            v.push((format!("OCR{n}B (active)"), self.ocr[1].to_string()));
            v.push((format!("OC{n}B"), self.oc[1].to_string()));
        }
        if self.c.c_unit.is_some() {
            v.push((format!("OCR{n}C (active)"), self.ocr[2].to_string()));
            v.push((format!("OC{n}C"), self.oc[2].to_string()));
        }
        if self.c.wide {
            v.push(("TEMP".into(), format!("0x{:02X}", self.temp)));
        }
        v
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
