//! General-purpose timers TIM2 (32-bit), TIM3/TIM4 and basic timers TIM6/TIM7 (RM0440 section 29-30).
//!
//! Event driven: the counter is `(cnt0, base)` -- the value `cnt0` at cycle `base`, advancing one
//! step every `tick` HCLK cycles ((PSC + 1) x timer clock ratio). Register accesses read the counter
//! lazily; the only scheduled event is the next update (overflow / underflow) or compare match, whose
//! exact cycle is computed from the reference, so PWM edges land on the right cycle.
//!
//! Modelled: CR1 (CEN, UDIS, URS, OPM, DIR, ARPE), DIER (UIE, CCxIE), SR (UIF, CCxIF), EGR (UG, CCxG),
//! CNT, PSC (shadowed: takes effect at the next update event, like silicon), ARR (ARPE preload),
//! CCR1-4 (OCxPE preload), CCMR1/2 output compare modes (frozen, active/inactive on match, toggle,
//! forced, PWM1/PWM2) and CCER (CCxE, CCxP) driving the alternate-function pin signal.
//!
//! Not modelled: input capture, center-aligned mode (treated as edge-aligned), external clock /
//! slave modes and triggers (SMCR is stored), repetition counter, DMA requests, one-pulse trigger.
//! Disabling the timer's RCC clock only blocks register access.

use mcs_core::arm::device::TimerInstance;

use crate::arm::bus::{Cx, Mmio};
use crate::arm::sys::sig;

use super::{lane_read, lane_write};

const CR1: u32 = 0x00;
const CR2: u32 = 0x04;
const SMCR: u32 = 0x08;
const DIER: u32 = 0x0c;
const SR: u32 = 0x10;
const EGR: u32 = 0x14;
const CCMR1: u32 = 0x18;
const CCMR2: u32 = 0x1c;
const CCER: u32 = 0x20;
const CNT: u32 = 0x24;
const PSC: u32 = 0x28;
const ARR: u32 = 0x2c;
const CCR1: u32 = 0x34;

const CEN: u32 = 1;
const UDIS: u32 = 2;
const URS: u32 = 4;
const OPM: u32 = 8;
const DIR: u32 = 16;
const ARPE: u32 = 128;

const SR_UIF: u32 = 1;
const EV: u8 = 0;

/// What the next event is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
    Update,
    Compare(usize),
}

pub struct Timer {
    n: u8,
    width: u8,
    channels: usize,
    apb: u8,
    irq: u32,
    en_reg: u8,
    en_bit: u8,
    cr1: u32,
    cr2: u32,
    smcr: u32,
    dier: u32,
    sr: u32,
    ccmr: [u32; 2],
    ccer: u32,
    psc: u32,
    psc_act: u32,
    arr: u64,
    arr_act: u64,
    ccr: [u32; 4],
    ccr_act: [u32; 4],
    /// Counter value at cycle `base`.
    cnt0: u64,
    base: u64,
    /// HCLK cycles per counter step at the time of the last rebase.
    tick: u64,
    running: bool,
    /// Compare channels already handled in the current counting period.
    cc_done: u8,
    /// OCxREF level for the modes that latch it on a match.
    oc: [u8; 4],
}

impl Timer {
    pub fn new(inst: &TimerInstance) -> Self {
        let mut t = Self {
            n: inst.name.trim_start_matches("TIM").parse().unwrap_or(0),
            width: inst.width,
            channels: inst.channels as usize,
            apb: inst.apb,
            irq: inst.irq as u32,
            en_reg: inst.enable.reg,
            en_bit: inst.enable.bit,
            cr1: 0,
            cr2: 0,
            smcr: 0,
            dier: 0,
            sr: 0,
            ccmr: [0; 2],
            ccer: 0,
            psc: 0,
            psc_act: 0,
            arr: 0,
            arr_act: 0,
            ccr: [0; 4],
            ccr_act: [0; 4],
            cnt0: 0,
            base: 0,
            tick: 1,
            running: false,
            cc_done: 0,
            oc: [0; 4],
        };
        t.set_reset_values();
        t
    }

    fn set_reset_values(&mut self) {
        let max = self.max();
        self.cr1 = 0;
        self.cr2 = 0;
        self.smcr = 0;
        self.dier = 0;
        self.sr = 0;
        self.ccmr = [0; 2];
        self.ccer = 0;
        self.psc = 0;
        self.psc_act = 0;
        self.arr = max;
        self.arr_act = max;
        self.ccr = [0; 4];
        self.ccr_act = [0; 4];
        self.cnt0 = 0;
        self.base = 0;
        self.running = false;
        self.cc_done = 0;
        self.oc = [0; 4];
    }

    #[inline]
    fn max(&self) -> u64 {
        if self.width == 32 { 0xffff_ffff } else { 0xffff }
    }

    #[inline]
    fn down(&self) -> bool {
        self.channels > 0 && self.cr1 & DIR != 0
    }

    fn compute_tick(&self, cx: &Cx) -> u64 {
        (self.psc_act as u64 + 1) * cx.sys.clk.timer_div(self.apb) as u64
    }

    /// Counter value at cycle `c`.
    fn cnt_at(&self, c: u64) -> u64 {
        if !self.running {
            return self.cnt0;
        }
        let ticks = c.saturating_sub(self.base) / self.tick;
        let period = self.arr_act + 1;
        if self.down() {
            let c0 = self.cnt0.min(self.arr_act);
            if ticks <= c0 { c0 - ticks } else { self.arr_act - (ticks - c0 - 1) % period }
        } else if self.cnt0 + ticks <= self.arr_act {
            self.cnt0 + ticks
        } else {
            (self.cnt0 + ticks - period) % period
        }
    }

    /// Re-bases the reference to the most recent counter step at or before `now` (keeps the
    /// sub-step phase).
    fn sync(&mut self, now: u64) {
        if self.running {
            let ticks = now.saturating_sub(self.base) / self.tick;
            self.cnt0 = self.cnt_at(now);
            self.base += ticks * self.tick;
        }
    }

    fn oc_mode(&self, ch: usize) -> u32 {
        self.ccmr[ch >> 1] >> (4 + 8 * (ch & 1)) & 7
    }

    /// Channel configured as output compare (CCxS = 0).
    fn is_output(&self, ch: usize) -> bool {
        ch < self.channels && self.ccmr[ch >> 1] >> (8 * (ch & 1)) & 3 == 0
    }

    /// Cycle of the next event after the reference: the update event and every pending compare.
    fn next_event(&self) -> Option<(u64, Next)> {
        if !self.running {
            return None;
        }
        let down = self.down();
        let steps_to_update = if down { self.cnt0 + 1 } else { self.arr_act + 1 - self.cnt0 };
        let mut best = (self.base + steps_to_update * self.tick, Next::Update);
        for ch in 0..self.channels {
            if self.cc_done >> ch & 1 != 0 || !self.is_output(ch) {
                continue;
            }
            let ccr = self.ccr_act[ch] as u64;
            let steps = if down {
                if ccr > self.cnt0 { continue } else { self.cnt0 - ccr }
            } else if ccr < self.cnt0 || ccr > self.arr_act {
                continue;
            } else {
                ccr - self.cnt0
            };
            let at = self.base + steps * self.tick;
            if at < best.0 {
                best = (at, Next::Compare(ch));
            }
        }
        Some(best)
    }

    fn reschedule(&self, cx: &mut Cx) {
        match self.next_event() {
            Some((at, _)) => cx.schedule(EV, at.max(cx.cycles)),
            None => cx.cancel(EV),
        }
    }

    /// Marks channels whose compare lies behind the counter as done for this period.
    fn recompute_cc_done(&mut self) {
        self.cc_done = 0;
        for ch in 0..self.channels {
            let ccr = self.ccr_act[ch] as u64;
            let behind = if self.down() { ccr > self.cnt0 } else { ccr <= self.cnt0 };
            if behind {
                self.cc_done |= 1 << ch;
            }
        }
    }

    fn ocref(&self, ch: usize, cnt: u64) -> u8 {
        let ccr = self.ccr_act[ch] as u64;
        match self.oc_mode(ch) {
            4 => 0,
            5 => 1,
            m @ (6 | 7) => {
                let pwm1 = if self.down() { cnt <= ccr } else { cnt < ccr };
                (pwm1 == (m == 6)) as u8
            }
            _ => self.oc[ch],
        }
    }

    /// Drives the pin signals of all output channels from OCxREF, CCxE and CCxP.
    fn drive_outputs(&mut self, cnt: u64, cycle: u64, cx: &mut Cx) {
        if self.n < 2 || self.n > 4 {
            return;
        }
        for ch in 0..self.channels {
            let on = self.ccer >> (4 * ch) & 1 != 0 && self.is_output(ch);
            let level = if on { self.ocref(ch, cnt) ^ (self.ccer >> (4 * ch + 1) & 1) as u8 } else { 0 };
            cx.sys.sig_out(sig::tim(self.n, ch as u8), level, cycle);
        }
    }

    fn update_irq(&self, cx: &mut Cx) {
        cx.set_irq_line(self.irq, self.sr & self.dier & 0x1f != 0);
    }

    /// Update event at cycle `at`: reload shadow registers, restart the period.
    fn update_event(&mut self, at: u64, cx: &mut Cx) {
        if self.cr1 & UDIS == 0 {
            self.psc_act = self.psc;
            self.arr_act = self.arr;
            self.ccr_act = self.ccr;
            self.sr |= SR_UIF;
        }
        self.tick = self.compute_tick(cx);
        self.cnt0 = if self.down() { self.arr_act } else { 0 };
        self.base = at;
        self.recompute_cc_done_after_update();
        if self.cr1 & OPM != 0 {
            self.running = false;
        }
    }

    /// After an update event the counter restarts: every channel compares again, including a
    /// compare value equal to the restart value (which matches immediately).
    fn recompute_cc_done_after_update(&mut self) {
        self.cc_done = 0;
        for ch in 0..self.channels {
            let ccr = self.ccr_act[ch] as u64;
            if (!self.down() && ccr > self.arr_act) || (self.down() && ccr > self.arr_act) {
                self.cc_done |= 1 << ch;
            }
        }
    }

    /// Processes every event due at or before `now`.
    fn run_events(&mut self, now: u64, cx: &mut Cx) {
        let mut guard = 0;
        while let Some((at, kind)) = self.next_event() {
            if at > now || guard > 100_000 {
                break;
            }
            guard += 1;
            match kind {
                Next::Update => {
                    self.update_event(at, cx);
                    let cnt = self.cnt0;
                    self.drive_outputs(cnt, at, cx);
                }
                Next::Compare(ch) => {
                    self.cc_done |= 1 << ch;
                    self.sr |= 2 << ch;
                    match self.oc_mode(ch) {
                        1 => self.oc[ch] = 1,
                        2 => self.oc[ch] = 0,
                        3 => self.oc[ch] ^= 1,
                        _ => {}
                    }
                    let cnt = self.ccr_act[ch] as u64;
                    self.drive_outputs(cnt, at, cx);
                }
            }
        }
        self.update_irq(cx);
        self.reschedule(cx);
    }

    fn start(&mut self, now: u64, cx: &mut Cx) {
        self.running = true;
        self.tick = self.compute_tick(cx);
        self.base = now;
        self.recompute_cc_done();
        self.reschedule(cx);
    }

    fn stop(&mut self, now: u64, cx: &mut Cx) {
        self.sync(now);
        self.running = false;
        cx.cancel(EV);
    }

    fn generate_update(&mut self, now: u64, cx: &mut Cx) {
        self.psc_act = self.psc;
        self.arr_act = self.arr;
        self.ccr_act = self.ccr;
        self.tick = self.compute_tick(cx);
        self.cnt0 = if self.down() { self.arr_act } else { 0 };
        self.base = now;
        if self.cr1 & URS == 0 && self.cr1 & UDIS == 0 {
            self.sr |= SR_UIF;
        }
        self.recompute_cc_done_after_update();
    }

    fn write_reg(&mut self, off: u32, v: u32, cx: &mut Cx) {
        let now = cx.cycles;
        match off {
            CR1 => {
                let old = self.cr1;
                self.sync(now);
                self.cr1 = v & if self.channels > 0 { 0x3ff } else { 0x8f };
                if self.cr1 & CEN != 0 && old & CEN == 0 {
                    self.start(now, cx);
                } else if self.cr1 & CEN == 0 && old & CEN != 0 {
                    self.stop(now, cx);
                } else {
                    if old & DIR != self.cr1 & DIR {
                        self.recompute_cc_done();
                    }
                    self.reschedule(cx);
                }
            }
            CR2 => self.cr2 = v,
            SMCR => self.smcr = v,
            DIER => {
                self.dier = v & 0x5f5f;
                self.update_irq(cx);
            }
            SR => {
                self.sr &= v | !0x1e5f;
                self.update_irq(cx);
            }
            EGR => {
                self.sync(now);
                if v & 1 != 0 {
                    self.generate_update(now, cx);
                }
                for ch in 0..self.channels {
                    if v >> (ch + 1) & 1 != 0 && self.is_output(ch) {
                        self.sr |= 2 << ch;
                        match self.oc_mode(ch) {
                            1 => self.oc[ch] = 1,
                            2 => self.oc[ch] = 0,
                            3 => self.oc[ch] ^= 1,
                            _ => {}
                        }
                    }
                }
                let cnt = self.cnt_at(now);
                self.drive_outputs(cnt, now, cx);
                self.update_irq(cx);
                self.reschedule(cx);
            }
            CCMR1 | CCMR2 if self.channels > 0 => {
                self.sync(now);
                self.ccmr[((off - CCMR1) >> 2) as usize] = v;
                let cnt = self.cnt_at(now);
                self.drive_outputs(cnt, now, cx);
                self.reschedule(cx);
            }
            CCER if self.channels > 0 => {
                self.ccer = v & 0x3333;
                let cnt = self.cnt_at(now);
                self.drive_outputs(cnt, now, cx);
            }
            CNT => {
                self.sync(now);
                self.cnt0 = (v as u64) & self.max();
                self.base = now;
                self.recompute_cc_done();
                let cnt = self.cnt0;
                self.drive_outputs(cnt, now, cx);
                self.reschedule(cx);
            }
            PSC => self.psc = v & 0xffff,
            ARR => {
                self.sync(now);
                self.arr = (v as u64) & self.max();
                if self.cr1 & ARPE == 0 {
                    self.arr_act = self.arr;
                    self.recompute_cc_done();
                    self.reschedule(cx);
                }
            }
            o if self.channels > 0 && (CCR1..CCR1 + 16).contains(&o) => {
                let ch = ((o - CCR1) >> 2) as usize;
                self.sync(now);
                let v = if self.width == 32 { v } else { v & 0xffff };
                self.ccr[ch] = v;
                // Without OCxPE the compare value is used immediately.
                if self.ccmr[ch >> 1] >> (3 + 8 * (ch & 1)) & 1 == 0 {
                    self.ccr_act[ch] = v;
                    let ccr = v as u64;
                    let cnt = self.cnt_at(now);
                    let behind = if self.down() { ccr > self.cnt0 } else { ccr <= self.cnt0 };
                    if behind {
                        self.cc_done |= 1 << ch;
                    } else {
                        self.cc_done &= !(1 << ch);
                    }
                    self.drive_outputs(cnt, now, cx);
                    self.reschedule(cx);
                }
            }
            _ => {}
        }
    }

    fn read_reg(&self, off: u32, now: u64) -> u32 {
        match off {
            CR1 => self.cr1,
            CR2 => self.cr2,
            SMCR => self.smcr,
            DIER => self.dier,
            SR => self.sr,
            CCMR1 if self.channels > 0 => self.ccmr[0],
            CCMR2 if self.channels > 0 => self.ccmr[1],
            CCER if self.channels > 0 => self.ccer,
            CNT => self.cnt_at(now) as u32,
            PSC => self.psc,
            ARR => self.arr as u32,
            o if self.channels > 0 && (CCR1..CCR1 + 16).contains(&o) => self.ccr[((o - CCR1) >> 2) as usize],
            _ => 0,
        }
    }
}

impl Mmio for Timer {
    fn read(&mut self, offset: u32, size: u8, cx: &mut Cx) -> u32 {
        if !cx.sys.clock_on(self.en_reg, self.en_bit) {
            return 0;
        }
        lane_read(self.read_reg(offset & !3, cx.cycles), offset, size)
    }

    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx) {
        if !cx.sys.clock_on(self.en_reg, self.en_bit) {
            return;
        }
        let off = offset & !3;
        let now = cx.cycles;
        let old = if off == EGR { 0 } else { self.read_reg(off, now) };
        let v = lane_write(old, offset, size, value);
        self.write_reg(off, v, cx);
    }

    fn peek(&mut self, offset: u32, cx: &mut Cx) -> u32 {
        self.read_reg(offset & !3, cx.cycles)
    }

    fn on_event(&mut self, _tag: u8, cx: &mut Cx) {
        let now = cx.cycles;
        self.run_events(now, cx);
    }

    fn on_clock_change(&mut self, cx: &mut Cx) {
        let now = cx.cycles;
        if self.running {
            self.sync(now);
            self.tick = self.compute_tick(cx);
            self.reschedule(cx);
        } else {
            self.tick = self.compute_tick(cx);
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        cx.cancel(EV);
        self.set_reset_values();
        self.tick = self.compute_tick(cx);
        self.update_irq(cx);
        let c = cx.cycles;
        self.drive_outputs(0, c, cx);
    }

    fn inspect(&self, cx: &Cx) -> Vec<(String, String)> {
        if !self.running && self.cnt0 == 0 && self.sr == 0 && self.dier == 0 {
            return Vec::new();
        }
        let tick_s = self.tick as f64 / cx.sys.clk.hclk_hz;
        let mut v = vec![
            ("Counter".into(), format!("{} / {}{}", self.cnt_at(cx.cycles), self.arr_act, if self.down() { " (down)" } else { "" })),
            ("State".into(), if self.running { "running".into() } else { "stopped".into() }),
            ("Prescaler".into(), format!("{} (step {:.3} us)", self.psc_act + 1, tick_s * 1e6)),
            ("Update period".into(), format!("{:.6} ms", (self.arr_act + 1) as f64 * tick_s * 1e3)),
        ];
        for ch in 0..self.channels {
            if self.ccer >> (4 * ch) & 1 != 0 {
                let mode = ["frozen", "active on match", "inactive on match", "toggle", "force inactive", "force active", "PWM1", "PWM2"][self.oc_mode(ch) as usize];
                v.push((format!("CH{}", ch + 1), format!("{mode}, CCR = {}", self.ccr_act[ch])));
            }
        }
        v
    }
}
