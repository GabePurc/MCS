//! TIMG0 / TIMG1: timer group with the 54-bit general-purpose timer T0 and the main system watchdog (ESP32-C3
//! TRM "Timer Group").
//!
//! T0: clocked by APB_CLK (or XTAL_CLK with `USE_XTAL`) through a 16-bit divider (`DIVIDER` 0 means 65536,
//! 1 means 2), counting up or down (`INCREASE`), started by `EN`. `T0UPDATE` latches the count into `T0LO/HI`,
//! `T0LOAD` loads `T0LOADLO/HI`. With `ALARM_EN` an alarm raises `T0_INT_RAW` when the count reaches
//! `T0ALARMLO/HI` (greater or equal when counting up, less or equal when counting down), clears `ALARM_EN` and,
//! with `AUTORELOAD`, reloads the load value. Interrupt matrix sources: 32 / 34 (T0), 33 / 35 (watchdog).
//!
//! The main system watchdog registers (`WDTCONFIG0-5`, `WDTFEED`, `WDTWPROTECT`) are stored and honour the write
//! protection key but the watchdog never counts or resets: it is disabled after direct boot (`WDT_EN` = 0).
//! The timer group runs only while its clock is enabled in `SYSTEM_PERIP_CLK_EN0` (bit 13 / 15).

use mcs_core::riscv::device::{RiscvDeviceSpec, RiscvTimgInstance};

use crate::riscv::bus::{Cx, Mmio};

use super::misc::RegFile;
use super::sys::Ticker;

const MASK54: u64 = (1 << 54) - 1;
const WPROTECT_KEY: u32 = 0x50d8_3aa1;

const T0CONFIG: u32 = 0x00;
const EN: u32 = 1 << 31;
const INCREASE: u32 = 1 << 30;
const AUTORELOAD: u32 = 1 << 29;
const ALARM_EN: u32 = 1 << 10;
const USE_XTAL: u32 = 1 << 9;

pub struct Timg {
    name: String,
    index: u8,
    t0_source: u8,
    wdt_source: u8,
    cfg: u32,
    t0: Ticker,
    latched: u64,
    alarm: u64,
    load: u64,
    int_ena: u32,
    int_raw: u32,
    regs: RegFile,
}

impl Timg {
    pub fn new(spec: &RiscvDeviceSpec, inst: &RiscvTimgInstance) -> Self {
        let regs = RegFile::from_spec(spec, &inst.name, inst.base, 0x100);
        Self { name: inst.name.clone(), index: inst.index, t0_source: inst.t0_source, wdt_source: inst.wdt_source, cfg: 0x6000_2000, t0: Ticker::new(), latched: 0, alarm: 0, load: 0, int_ena: 0, int_raw: 0, regs }
    }

    fn clock_bit(&self) -> u8 {
        if self.index == 0 {
            13
        } else {
            15
        }
    }

    fn divider(&self) -> u64 {
        match self.cfg >> 13 & 0xffff {
            0 => 65536,
            1 => 2,
            d => d as u64,
        }
    }

    fn running(&self, cx: &Cx) -> bool {
        self.cfg & EN != 0 && cx.sys.clock_on(0, self.clock_bit())
    }

    fn retime(&mut self, cx: &mut Cx) {
        let src = if self.cfg & USE_XTAL != 0 { cx.sys.clk.xtal } else { cx.sys.clk.apb };
        let ratio = cx.sys.clk.cpu.cycles_per_tick(src, self.divider());
        let (now, run) = (cx.cycles, self.running(cx));
        self.t0.down = self.cfg & INCREASE == 0;
        self.t0.retime(now, ratio, run);
        self.schedule(cx);
    }

    fn schedule(&mut self, cx: &mut Cx) {
        cx.cancel(0);
        if self.cfg & ALARM_EN == 0 || !self.running(cx) {
            return;
        }
        let now = cx.cycles;
        let cur = self.t0.value_at(now) & MASK54;
        let down = self.t0.down;
        if (!down && cur >= self.alarm) || (down && cur <= self.alarm) {
            cx.schedule(0, now);
        } else if let Some(at) = self.t0.cycle_of(now, self.alarm, MASK54) {
            cx.schedule(0, at);
        }
    }

    fn irq(&self, cx: &mut Cx) {
        let st = self.int_raw & self.int_ena;
        cx.irq_source(self.t0_source, st & 1 != 0);
        cx.irq_source(self.wdt_source, st & 2 != 0);
    }
}

impl Mmio for Timg {
    fn read(&mut self, off: u32, _size: u8, _cx: &mut Cx) -> u32 {
        match off {
            T0CONFIG => self.cfg,
            0x04 => self.latched as u32,
            0x08 => (self.latched >> 32) as u32 & 0x3f_ffff,
            0x10 => self.alarm as u32,
            0x14 => (self.alarm >> 32) as u32,
            0x18 => self.load as u32,
            0x1c => (self.load >> 32) as u32,
            0x70 => self.int_ena,
            0x74 => self.int_raw,
            0x78 => self.int_raw & self.int_ena,
            0x0c | 0x20 | 0x7c => 0,
            _ => self.regs.get(off),
        }
    }

    fn write(&mut self, off: u32, _size: u8, v: u32, cx: &mut Cx) {
        let now = cx.cycles;
        match off {
            T0CONFIG => {
                self.cfg = v & !(1 << 12);
                self.retime(cx);
            }
            0x0c => {
                if v >> 31 != 0 {
                    self.latched = self.t0.value_at(now) & MASK54;
                }
            }
            0x10 => {
                self.alarm = (self.alarm & !0xffff_ffff) | v as u64;
                self.schedule(cx);
            }
            0x14 => {
                self.alarm = (self.alarm & 0xffff_ffff) | ((v & 0x3f_ffff) as u64) << 32;
                self.schedule(cx);
            }
            0x18 => self.load = (self.load & !0xffff_ffff) | v as u64,
            0x1c => self.load = (self.load & 0xffff_ffff) | ((v & 0x3f_ffff) as u64) << 32,
            0x20 => {
                self.t0.set(now, self.load);
                self.schedule(cx);
            }
            0x70 => {
                self.int_ena = v & 3;
                self.irq(cx);
            }
            0x74 => {
                self.int_raw = (self.int_raw & !3) | (v & 3);
                self.irq(cx);
            }
            0x7c => {
                self.int_raw &= !(v & 3);
                self.irq(cx);
            }
            0x78 => {}
            // WDTCONFIG0-5, WDTFEED: only while the write protection is open.
            0x48..=0x60 => {
                if self.regs.get(0x64) == WPROTECT_KEY {
                    self.regs.put(off, v);
                }
            }
            _ => self.regs.put(off, v),
        }
    }

    fn on_event(&mut self, _tag: u8, cx: &mut Cx) {
        self.int_raw |= 1;
        self.cfg &= !ALARM_EN;
        if self.cfg & AUTORELOAD != 0 {
            let at = cx.cycles;
            self.t0.set(at, self.load);
        }
        self.irq(cx);
    }

    fn on_clock_change(&mut self, cx: &mut Cx) {
        self.retime(cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        cx.cancel(0);
        self.regs.reset();
        self.cfg = 0x6000_2000;
        self.t0 = Ticker::new();
        self.latched = 0;
        self.alarm = 0;
        self.load = 0;
        self.int_ena = 0;
        self.int_raw = 0;
        self.retime(cx);
        self.irq(cx);
    }

    fn inspect(&self, cx: &Cx) -> Vec<(String, String)> {
        let v = self.t0.value_at(cx.cycles) & MASK54;
        vec![
            (format!("{} T0", self.name), format!("{v}{}", if self.cfg & EN != 0 { "" } else { " (stopped)" })),
            ("Divider".into(), self.divider().to_string()),
            ("Watchdog".into(), "disabled (not simulated)".into()),
        ]
    }
}
