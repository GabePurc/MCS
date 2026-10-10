//! SYSTIMER (ESP32-C3 TRM "System Timer"): two 52-bit counters (unit 0 / unit 1) counting at 16 MHz
//! (XTAL_CLK / 2.5, independent of the CPU clock) and three comparators with one-shot (target) and periodic
//! alarm modes. Alarms raise interrupt matrix sources 37 - 39.
//!
//! Modelled: `CONF` (`UNITn_WORK_EN` enable the counters, `TARGETn_WORK_EN` the comparators; `CLK_EN` is not
//! required for register access), `UNITn_OP.UPDATE` (latch the count into `UNITn_VALUE`), `UNITn_LOAD_HI/LO`
//! with `UNITn_LOAD`, `TARGETn_HI/LO`, `TARGETn_CONF` (`PERIOD`, `PERIOD_MODE`, `TIMER_UNIT_SEL`), `COMPn_LOAD`,
//! `INT_RAW/ENA/ST/CLR`. The stall-on-debug bits are stored and ignored.
//!
//! Semantics (TRM, with assumptions where the manual is not explicit): an alarm fires when the selected counter
//! is greater than or equal to the target when the comparator is loaded or later reaches it; in period mode
//! `COMPn_LOAD` arms the first alarm one `PERIOD` after the counter value at that moment and the comparator
//! then advances by `PERIOD` after each alarm (assumption: TARGETn_HI/LO are not used in period mode).
//! `UNITn_VALUE_VALID` always reads 1.

use crate::riscv::bus::{Cx, Mmio};

use super::sys::{Ticker, SYSTIMER_HZ};

const MASK52: u64 = (1 << 52) - 1;

const CONF: u32 = 0x00;
const UNIT0_OP: u32 = 0x04;
const UNIT1_OP: u32 = 0x08;
const INT_ENA: u32 = 0x64;
const INT_RAW: u32 = 0x68;
const INT_CLR: u32 = 0x6c;
const INT_ST: u32 = 0x70;

const SRC_TARGET0: u8 = 37;

/// Event tags: alarm of comparator n.
const EV_ALARM: u8 = 0;

#[derive(Clone, Copy, Default)]
struct Comp {
    /// Target value (HI << 32 | LO).
    target: u64,
    conf: u32,
    /// Comparator value armed by `COMPn_LOAD`; `None` = disarmed (fired in one-shot mode).
    armed: Option<u64>,
    period_mode_target: u64,
}

pub struct SysTimer {
    conf: u32,
    unit: [Ticker; 2],
    load_hi: [u32; 2],
    load_lo: [u32; 2],
    latched: [u64; 2],
    comp: [Comp; 3],
    int_ena: u32,
    int_raw: u32,
    other: [u32; 8],
}

impl SysTimer {
    pub fn new() -> Self {
        let mut s = Self { conf: 0x4600_0000, unit: [Ticker::new(); 2], load_hi: [0; 2], load_lo: [0; 2], latched: [0; 2], comp: [Comp::default(); 3], int_ena: 0, int_raw: 0, other: [0; 8] };
        s.other[7] = 0x0200_6171; // DATE is at 0xfc: other[(0xfc - 0xe0) / 4]
        s
    }

    fn ratio(cx: &Cx) -> (u64, u64) {
        cx.sys.clk.cpu.cycles_per_tick(SYSTIMER_HZ, 1)
    }

    fn unit_run(&self, u: usize) -> bool {
        self.conf >> (30 - u) & 1 != 0
    }

    fn comp_enabled(&self, n: usize) -> bool {
        self.conf >> (24 - n) & 1 != 0
    }

    fn unit_of(&self, n: usize) -> usize {
        (self.comp[n].conf >> 31) as usize
    }

    fn irq(&self, cx: &mut Cx) {
        let st = self.int_raw & self.int_ena;
        for n in 0..3 {
            cx.irq_source(SRC_TARGET0 + n, st >> n & 1 != 0);
        }
    }

    /// (Re)schedules the alarm event of comparator `n`.
    fn schedule(&mut self, n: usize, cx: &mut Cx) {
        cx.cancel(n as u8 + EV_ALARM);
        let Some(target) = self.comp[n].armed else { return };
        if !self.comp_enabled(n) {
            return;
        }
        let u = self.unit_of(n);
        let now = cx.cycles;
        let cur = self.unit[u].value_at(now) & MASK52;
        if cur >= target {
            cx.schedule(n as u8, now);
        } else if let Some(at) = self.unit[u].cycle_of(now, target, MASK52) {
            cx.schedule(n as u8, at);
        }
    }

    fn reschedule_all(&mut self, cx: &mut Cx) {
        for n in 0..3 {
            self.schedule(n, cx);
        }
    }

    fn retime(&mut self, cx: &mut Cx) {
        let (r, now) = (Self::ratio(cx), cx.cycles);
        for u in 0..2 {
            let run = self.unit_run(u);
            self.unit[u].retime(now, r, run);
        }
        self.reschedule_all(cx);
    }

    fn comp_load(&mut self, n: usize, cx: &mut Cx) {
        let c = &mut self.comp[n];
        let u = (c.conf >> 31) as usize;
        let period = (c.conf & 0x03ff_ffff) as u64;
        if c.conf >> 30 & 1 != 0 {
            let cur = self.unit[u].value_at(cx.cycles) & MASK52;
            let t = (cur + period.max(1)) & MASK52;
            c.armed = Some(t);
            c.period_mode_target = t;
        } else {
            c.armed = Some(c.target & MASK52);
        }
        self.schedule(n, cx);
    }
}

impl Default for SysTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl Mmio for SysTimer {
    fn read(&mut self, off: u32, _size: u8, cx: &mut Cx) -> u32 {
        match off {
            CONF => self.conf,
            UNIT0_OP | UNIT1_OP => 1 << 29,
            0x1c => (self.comp[0].target >> 32) as u32,
            0x20 => self.comp[0].target as u32,
            0x24 => (self.comp[1].target >> 32) as u32,
            0x28 => self.comp[1].target as u32,
            0x2c => (self.comp[2].target >> 32) as u32,
            0x30 => self.comp[2].target as u32,
            0x34 => self.comp[0].conf,
            0x38 => self.comp[1].conf,
            0x3c => self.comp[2].conf,
            0x40 => (self.latched[0] >> 32) as u32,
            0x44 => self.latched[0] as u32,
            0x48 => (self.latched[1] >> 32) as u32,
            0x4c => self.latched[1] as u32,
            0x0c => self.load_hi[0],
            0x10 => self.load_lo[0],
            0x14 => self.load_hi[1],
            0x18 => self.load_lo[1],
            INT_ENA => self.int_ena,
            INT_RAW => self.int_raw,
            INT_ST => self.int_raw & self.int_ena,
            0xe0..=0xfc => self.other[((off - 0xe0) / 4) as usize],
            _ => {
                let _ = cx;
                0
            }
        }
    }

    fn write(&mut self, off: u32, _size: u8, v: u32, cx: &mut Cx) {
        let now = cx.cycles;
        match off {
            CONF => {
                self.conf = v;
                self.retime(cx);
            }
            UNIT0_OP | UNIT1_OP => {
                if v >> 30 & 1 != 0 {
                    let u = (off == UNIT1_OP) as usize;
                    self.latched[u] = self.unit[u].value_at(now) & MASK52;
                }
            }
            0x0c => self.load_hi[0] = v & 0xf_ffff,
            0x10 => self.load_lo[0] = v,
            0x14 => self.load_hi[1] = v & 0xf_ffff,
            0x18 => self.load_lo[1] = v,
            0x1c | 0x24 | 0x2c => {
                let n = ((off - 0x1c) / 8) as usize;
                self.comp[n].target = (self.comp[n].target & 0xffff_ffff) | ((v & 0xf_ffff) as u64) << 32;
            }
            0x20 | 0x28 | 0x30 => {
                let n = ((off - 0x20) / 8) as usize;
                self.comp[n].target = (self.comp[n].target & !0xffff_ffff) | v as u64;
            }
            0x34 | 0x38 | 0x3c => {
                let n = ((off - 0x34) / 4) as usize;
                self.comp[n].conf = v & 0xc3ff_ffff;
            }
            0x50 | 0x54 | 0x58 => {
                if v & 1 != 0 {
                    self.comp_load(((off - 0x50) / 4) as usize, cx);
                }
            }
            0x5c | 0x60 => {
                if v & 1 != 0 {
                    let u = ((off - 0x5c) / 4) as usize;
                    let val = (self.load_hi[u] as u64) << 32 | self.load_lo[u] as u64;
                    self.unit[u].set(now, val & MASK52);
                    self.reschedule_all(cx);
                }
            }
            INT_ENA => {
                self.int_ena = v & 7;
                self.irq(cx);
            }
            INT_RAW => {
                self.int_raw = (self.int_raw & !7) | (v & 7);
                self.irq(cx);
            }
            INT_CLR => {
                self.int_raw &= !(v & 7);
                self.irq(cx);
            }
            0xe0..=0xfc => self.other[((off - 0xe0) / 4) as usize] = v,
            _ => {}
        }
    }

    fn on_event(&mut self, tag: u8, cx: &mut Cx) {
        let n = tag as usize;
        if n >= 3 {
            return;
        }
        self.int_raw |= 1 << n;
        let c = &mut self.comp[n];
        if c.conf >> 30 & 1 != 0 {
            let period = ((c.conf & 0x03ff_ffff) as u64).max(1);
            let next = (c.armed.unwrap_or(0) + period) & MASK52;
            c.armed = Some(next);
            self.schedule(n, cx);
        } else {
            c.armed = None;
        }
        self.irq(cx);
    }

    fn on_clock_change(&mut self, cx: &mut Cx) {
        self.retime(cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        for n in 0..3 {
            cx.cancel(n);
        }
        *self = Self::new();
        let (r, now) = (Self::ratio(cx), cx.cycles);
        // UNIT0_WORK_EN is set out of reset.
        let run = self.unit_run(0);
        self.unit[0].retime(now, r, run);
        self.unit[1].retime(now, r, false);
        self.irq(cx);
    }

    fn inspect(&self, cx: &Cx) -> Vec<(String, String)> {
        let now = cx.cycles;
        vec![
            ("Unit 0".into(), format!("{} ticks{}", self.unit[0].value_at(now) & MASK52, if self.unit_run(0) { "" } else { " (stopped)" })),
            ("Unit 1".into(), format!("{} ticks{}", self.unit[1].value_at(now) & MASK52, if self.unit_run(1) { "" } else { " (stopped)" })),
        ]
    }
}
