//! SysTick timer (CSR 0xE000E010, RVR +4, CVR +8, CALIB +C).
//!
//! Reference: ARM DDI 0403E.e B3.3 and Cortex-M4 Devices Generic User Guide (ARM DUI 0553) 4.4.
//!
//! Event driven: nothing runs per cycle. The counter is described by `(base_cycle, base_val)`; the
//! value at any cycle follows from the elapsed ticks (`div` CPU cycles per tick), and the only
//! scheduled event is the next wrap to zero, which sets COUNTFLAG and (with TICKINT) pends the
//! SysTick exception. Register accesses re-synchronise `(base_cycle, base_val)` at the current
//! cycle; the partial tick of an external reference clock (`div` > 1) is dropped at that moment.

use crate::scheduler::{EventKey, Scheduler};

use super::nvic::{Nvic, EXC_SYSTICK};

pub const CSR_ENABLE: u32 = 1;
pub const CSR_TICKINT: u32 = 2;
pub const CSR_CLKSOURCE: u32 = 4;
pub const CSR_COUNTFLAG: u32 = 1 << 16;

/// Scheduler owner id of the SysTick event.
pub const SYSTICK_OWNER: u8 = 0xff;
const KEY: EventKey = EventKey { owner: SYSTICK_OWNER, tag: 0 };

pub struct SysTick {
    pub enabled: bool,
    pub tickint: bool,
    pub clksource: bool,
    pub reload: u32,
    base_cycle: u64,
    base_val: u32,
    pub countflag: bool,
    /// CPU cycles per tick of the external reference clock (CLKSOURCE = 0).
    pub ext_div: u32,
    pub calib: u32,
}

impl SysTick {
    pub fn new(ext_div: u32, calib: u32) -> Self {
        Self {
            enabled: false,
            tickint: false,
            clksource: false,
            reload: 0,
            base_cycle: 0,
            base_val: 0,
            countflag: false,
            ext_div: ext_div.max(1),
            calib,
        }
    }

    pub fn reset(&mut self, sched: &mut Scheduler) {
        sched.cancel(KEY);
        *self = SysTick::new(self.ext_div, self.calib);
    }

    #[inline]
    fn div(&self) -> u64 {
        if self.clksource {
            1
        } else {
            self.ext_div as u64
        }
    }

    /// Current counter value (CVR) at `cycles`.
    pub fn value_at(&self, cycles: u64) -> u32 {
        if !self.enabled {
            return self.base_val;
        }
        let t = (cycles - self.base_cycle) / self.div();
        let v = self.base_val as u64;
        if t <= v {
            (v - t) as u32
        } else if self.reload == 0 {
            0
        } else {
            let period = self.reload as u64 + 1;
            self.reload - ((t - v - 1) % period) as u32
        }
    }

    /// Re-synchronises the base to `now` and (re)schedules the next wrap.
    fn resync(&mut self, now: u64, sched: &mut Scheduler) {
        self.base_val = self.value_at(now);
        self.base_cycle = now;
        self.schedule(sched);
    }

    fn schedule(&mut self, sched: &mut Scheduler) {
        if !self.enabled {
            sched.cancel(KEY);
            return;
        }
        let ticks = if self.base_val > 0 {
            self.base_val as u64
        } else if self.reload > 0 {
            self.reload as u64 + 1
        } else {
            sched.cancel(KEY);
            return;
        };
        sched.at(KEY, self.base_cycle + ticks * self.div());
    }

    pub fn read_csr(&mut self) -> u32 {
        let v = self.enabled as u32
            | (self.tickint as u32) << 1
            | (self.clksource as u32) << 2
            | (self.countflag as u32) << 16;
        self.countflag = false;
        v
    }

    pub fn write_csr(&mut self, v: u32, now: u64, sched: &mut Scheduler) {
        // Bring the counter to `now` using the old clock source before changing it.
        self.resync(now, sched);
        self.enabled = v & CSR_ENABLE != 0;
        self.tickint = v & CSR_TICKINT != 0;
        self.clksource = v & CSR_CLKSOURCE != 0;
        self.base_cycle = now;
        self.schedule(sched);
    }

    pub fn write_rvr(&mut self, v: u32, now: u64, sched: &mut Scheduler) {
        self.resync(now, sched);
        self.reload = v & 0x00ff_ffff;
        self.schedule(sched);
    }

    pub fn write_cvr(&mut self, now: u64, sched: &mut Scheduler) {
        self.base_val = 0;
        self.base_cycle = now;
        self.countflag = false;
        self.schedule(sched);
    }

    /// The wrap event at cycle `at` is due.
    pub fn on_event(&mut self, at: u64, sched: &mut Scheduler, nvic: &mut Nvic) {
        self.countflag = true;
        if self.tickint {
            nvic.set_sys_pending(EXC_SYSTICK);
        }
        self.base_val = 0;
        self.base_cycle = at;
        self.schedule(sched);
    }
}
