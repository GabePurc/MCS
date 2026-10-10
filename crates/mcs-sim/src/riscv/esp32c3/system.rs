//! SYSTEM (clock selection, peripheral clock enable / reset), the interrupt matrix register block
//! (INTERRUPT_CORE0) and RTC_CNTL (stored registers, software reset).
//!
//! SYSTEM: `SYSCLK_CONF` (`SOC_CLK_SEL`, `PRE_DIV_CNT`) and `CPU_PER_CONF` (`CPUPERIOD_SEL`) select the CPU
//! clock as described in [`Clocks::from_regs`]; the clock-enable and reset registers are stored and a rising
//! `PERIP_RST_EN` bit resets the peripheral wired to it; `CPU_INTR_FROM_CPU_n` bit 0 drives interrupt matrix
//! source 50 + n (software interrupts).
//!
//! RTC_CNTL: registers are stored. The RTC watchdog (`WDTCONFIG0`), the super watchdog (`SWD_CONF`) and the
//! timer-group main system watchdogs never reset the chip: they are disabled after direct boot (their reset
//! values have `WDT_EN` = 0) and enabling them has no effect. `OPTIONS0.SW_SYS_RST` / `SW_PROCPU_RST` reset the
//! system.

use mcs_core::riscv::device::RiscvDeviceSpec;

use crate::riscv::bus::{Cx, Mmio};

use super::misc::RegFile;
use super::sys::{Clocks, NSRC};

const CPU_PER_CONF: u32 = 0x08;
const PERIP_CLK_EN0: u32 = 0x10;
const PERIP_CLK_EN1: u32 = 0x14;
const PERIP_RST_EN0: u32 = 0x18;
const PERIP_RST_EN1: u32 = 0x1c;
const CPU_INTR_FROM_CPU_0: u32 = 0x28;
const SYSCLK_CONF: u32 = 0x58;

pub struct System {
    regs: RegFile,
}

impl System {
    pub fn new(spec: &RiscvDeviceSpec) -> Self {
        Self { regs: RegFile::from_spec(spec, "SYSTEM", spec.peripheral_set.system_base, 0x1000) }
    }

    fn sync(&self, cx: &mut Cx) {
        let r = &mut cx.sys.regs;
        r.clk_en0 = self.regs.get(PERIP_CLK_EN0);
        r.clk_en1 = self.regs.get(PERIP_CLK_EN1);
        r.rst0 = self.regs.get(PERIP_RST_EN0);
        r.rst1 = self.regs.get(PERIP_RST_EN1);
        r.cpu_per_conf = self.regs.get(CPU_PER_CONF);
        r.sysclk_conf = self.regs.get(SYSCLK_CONF);
    }

    fn clocks(&self, cx: &mut Cx) {
        let c = Clocks::from_regs(self.regs.get(SYSCLK_CONF), self.regs.get(CPU_PER_CONF));
        let now = cx.cycles;
        cx.sys.set_clocks(c, now);
    }
}

impl Mmio for System {
    fn read(&mut self, off: u32, _size: u8, _cx: &mut Cx) -> u32 {
        self.regs.get(off)
    }

    fn write(&mut self, off: u32, _size: u8, v: u32, cx: &mut Cx) {
        match off {
            PERIP_RST_EN0 | PERIP_RST_EN1 => {
                let old = self.regs.get(off);
                self.regs.put(off, v);
                let reg = (off == PERIP_RST_EN1) as u8;
                let mut rising = v & !old;
                while rising != 0 {
                    let b = rising.trailing_zeros() as u8;
                    rising &= rising - 1;
                    cx.sys.resets.push((reg, b));
                    cx.sys.attn = true;
                }
                self.sync(cx);
            }
            CPU_PER_CONF | SYSCLK_CONF => {
                self.regs.put(off, v);
                self.sync(cx);
                self.clocks(cx);
            }
            PERIP_CLK_EN0 | PERIP_CLK_EN1 => {
                self.regs.put(off, v);
                self.sync(cx);
            }
            o @ 0x28..=0x34 => {
                self.regs.put(o, v);
                let n = (o - CPU_INTR_FROM_CPU_0) / 4;
                cx.irq_source(50 + n as u8, v & 1 != 0);
            }
            _ => self.regs.put(off, v),
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.regs.reset();
        self.sync(cx);
        self.clocks(cx);
        for n in 0..4 {
            cx.irq_source(50 + n, false);
        }
    }

    fn inspect(&self, cx: &Cx) -> Vec<(String, String)> {
        let src = match (self.regs.get(SYSCLK_CONF) >> 10) & 3 {
            1 => "PLL",
            2 => "RC_FAST",
            _ => "XTAL",
        };
        vec![
            ("CPU clock".into(), format!("{:.3} MHz ({src})", cx.sys.clk.cpu.as_f64() / 1e6)),
            ("APB clock".into(), format!("{:.3} MHz", cx.sys.clk.apb.as_f64() / 1e6)),
        ]
    }
}

/// INTERRUPT_CORE0: the register face of [`Intc`](super::sys::Intc).
pub struct IntMatrix {
    regs: RegFile,
    clear: u32,
}

impl IntMatrix {
    pub fn new(spec: &RiscvDeviceSpec) -> Self {
        Self { regs: RegFile::from_spec(spec, "INTERRUPT_CORE0", spec.peripheral_set.intc_base, 0x800), clear: 0 }
    }
}

impl Mmio for IntMatrix {
    fn read(&mut self, off: u32, _size: u8, cx: &mut Cx) -> u32 {
        let i = &cx.sys.intc;
        match off {
            0xf8 => i.source_status() as u32,
            0xfc => (i.source_status() >> 32) as u32,
            0x104 => i.enable,
            0x108 => i.ty,
            0x10c => self.clear,
            0x110 => i.pending(),
            0x114..=0x190 => i.pri[((off - 0x114) / 4) as usize] as u32,
            0x194 => i.thresh as u32,
            _ => self.regs.get(off),
        }
    }

    fn write(&mut self, off: u32, _size: u8, v: u32, cx: &mut Cx) {
        match off {
            0x00..=0xf4 => {
                let src = (off / 4) as usize;
                if src < NSRC {
                    self.regs.put(off, v & 0x1f);
                    cx.sys.intc.set_map(src, (v & 0x1f) as u8);
                    cx.irq_update();
                }
            }
            0xf8 | 0xfc | 0x110 => {}
            0x104 => {
                cx.sys.intc.enable = v & !1;
                cx.irq_update();
            }
            0x108 => {
                cx.sys.intc.ty = v & !1;
                cx.irq_update();
            }
            0x10c => {
                let rising = v & !self.clear;
                self.clear = v;
                cx.sys.intc.clear(rising);
                cx.irq_update();
            }
            0x114..=0x190 => {
                cx.sys.intc.pri[((off - 0x114) / 4) as usize] = (v & 15) as u8;
                cx.irq_update();
            }
            0x194 => {
                cx.sys.intc.thresh = (v & 15) as u8;
                cx.irq_update();
            }
            _ => self.regs.put(off, v),
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.regs.reset();
        self.clear = 0;
        cx.sys.intc.reset();
        cx.irq_update();
    }

    fn inspect(&self, cx: &Cx) -> Vec<(String, String)> {
        let i = &cx.sys.intc;
        vec![
            ("Enabled".into(), format!("{:#010x}", i.enable)),
            ("Pending".into(), format!("{:#010x}", i.pending())),
            ("Threshold".into(), i.thresh.to_string()),
        ]
    }
}

/// RTC_CNTL (0x60008000); the EFUSE block (+0x800) reads as zero.
pub struct Rtc {
    regs: RegFile,
}

impl Rtc {
    pub fn new(spec: &RiscvDeviceSpec) -> Self {
        Self { regs: RegFile::from_spec(spec, "RTC_CNTL", spec.peripheral_set.rtc_base, 0x200) }
    }
}

impl Mmio for Rtc {
    fn read(&mut self, off: u32, _size: u8, _cx: &mut Cx) -> u32 {
        if off >= 0x200 {
            0
        } else {
            self.regs.get(off)
        }
    }

    fn write(&mut self, off: u32, _size: u8, v: u32, cx: &mut Cx) {
        if off >= 0x200 {
            return;
        }
        if off == 0 && v & ((1 << 31) | (1 << 5)) != 0 {
            // SW_SYS_RST / SW_PROCPU_RST
            cx.sys.reset_req = true;
            cx.request_stop();
            return;
        }
        self.regs.put(off, v);
    }

    fn reset(&mut self, _cx: &mut Cx) {
        self.regs.reset();
    }
}
