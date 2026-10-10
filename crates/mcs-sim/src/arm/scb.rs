//! System Control Block and the rest of the System Control Space dispatcher (0xE000_E000).
//!
//! Registers: ACTLR, SysTick (CSR/RVR/CVR/CALIB), NVIC, CPUID, ICSR, VTOR, AIRCR, SCR, CCR,
//! SHPR1-3, SHCSR, CFSR, HFSR, DFSR, MMFAR, BFAR, AFSR, CPACR and a read-as-zero MPU TYPE; plus the
//! DWT CTRL/CYCCNT pair (0xE000_1000) many STM32 programs use for cycle counting.
//!
//! References: ARM DDI 0403E.e B3.2 (SCB), Cortex-M4 Devices Generic User Guide (ARM DUI 0553)
//! chapter 4.

use super::machine::Machine;
use super::nvic::*;

pub const CCR_NONBASETHRDENA: u32 = 1 << 0;
pub const CCR_USERSETMPEND: u32 = 1 << 1;
pub const CCR_UNALIGN_TRP: u32 = 1 << 3;
pub const CCR_DIV_0_TRP: u32 = 1 << 4;
pub const CCR_STKALIGN: u32 = 1 << 9;

pub const SHCSR_MEMFAULTENA: u32 = 1 << 16;
pub const SHCSR_BUSFAULTENA: u32 = 1 << 17;
pub const SHCSR_USGFAULTENA: u32 = 1 << 18;

/// CFSR.UFSR bits.
pub const UFSR_UNDEFINSTR: u32 = 1 << 16;
pub const UFSR_INVSTATE: u32 = 1 << 17;
pub const UFSR_INVPC: u32 = 1 << 18;
pub const UFSR_NOCP: u32 = 1 << 19;
pub const UFSR_UNALIGNED: u32 = 1 << 24;
pub const UFSR_DIVBYZERO: u32 = 1 << 25;
/// CFSR.BFSR bits.
pub const BFSR_IBUSERR: u32 = 1 << 8;
pub const BFSR_PRECISERR: u32 = 1 << 9;
pub const BFSR_UNSTKERR: u32 = 1 << 11;
pub const BFSR_STKERR: u32 = 1 << 12;
pub const BFSR_BFARVALID: u32 = 1 << 15;
/// HFSR bits.
pub const HFSR_VECTTBL: u32 = 1 << 1;
pub const HFSR_FORCED: u32 = 1 << 30;

pub struct Scb {
    pub cpuid: u32,
    pub vtor: u32,
    pub scr: u32,
    pub ccr: u32,
    pub shcsr: u32,
    pub cfsr: u32,
    pub hfsr: u32,
    pub dfsr: u32,
    pub mmfar: u32,
    pub bfar: u32,
    pub afsr: u32,
    pub cpacr: u32,
    pub actlr: u32,
    pub dwt_ctrl: u32,
    pub dwt_off: u64,
}

impl Scb {
    pub fn new(cpuid: u32) -> Self {
        Self {
            cpuid,
            vtor: 0,
            scr: 0,
            ccr: CCR_STKALIGN,
            shcsr: 0,
            cfsr: 0,
            hfsr: 0,
            dfsr: 0,
            mmfar: 0,
            bfar: 0,
            afsr: 0,
            cpacr: 0,
            actlr: 0,
            dwt_ctrl: 0,
            dwt_off: 0,
        }
    }
}

impl Machine {
    /// Reads a System Control Space / PPB register (any size; sub-word reads extract bytes).
    pub(crate) fn ppb_read(&mut self, addr: u32, size: u32) -> Option<u32> {
        let word = self.ppb_read_word(addr & !3)?;
        let sh = (addr & 3) * 8;
        Some(match size {
            4 => word,
            2 => (word >> sh) & 0xffff,
            _ => (word >> sh) & 0xff,
        })
    }

    fn ppb_read_word(&mut self, addr: u32) -> Option<u32> {
        let cycles = self.cpu.cycles;
        if addr & 0xfffff000 == 0xe000_1000 {
            return Some(match addr & 0xfff {
                0 => self.scb.dwt_ctrl,
                4 => cycles.wrapping_sub(self.scb.dwt_off) as u32,
                _ => 0,
            });
        }
        if addr & 0xfffff000 != 0xe000_e000 {
            return Some(0); // ITM / FPB / reserved: RAZ
        }
        let off = addr & 0xfff;
        Some(match off {
            0x008 => self.scb.actlr,
            0x010 => self.systick.read_csr(),
            0x014 => self.systick.reload,
            0x018 => self.systick.value_at(cycles),
            0x01c => self.systick.calib,
            0x100..=0x4ef => self.nvic.read_word(off),
            0xd00 => self.scb.cpuid,
            0xd04 => {
                let n = &self.nvic;
                let isr_pending = (0..((n.nirq as usize + 31) >> 5)).any(|w| n.pending[w] != 0);
                (n.sys_pending >> EXC_NMI & 1) << 31
                    | (n.sys_pending >> EXC_PENDSV & 1) << 28
                    | (n.sys_pending >> EXC_SYSTICK & 1) << 26
                    | (isr_pending as u32) << 22
                    | (n.vect_pending() as u32) << 12
                    | ((self.nest.len() <= 1) as u32) << 11
                    | self.cpu.ipsr as u32
            }
            0xd08 => self.scb.vtor,
            0xd0c => 0xfa05_0000 | (self.nvic.prigroup as u32) << 8,
            0xd10 => self.scb.scr,
            0xd14 => self.scb.ccr,
            0xd18..=0xd23 => {
                let first = 4 + (off - 0xd18);
                (0..4).fold(0, |v, k| v | (self.nvic.prio[(first + k) as usize] as u32) << (8 * k))
            }
            0xd24 => {
                let n = &self.nvic;
                let act = |e: u16, b: u32| (n.sys_active >> e & 1) << b;
                let pend = |e: u16, b: u32| (n.sys_pending >> e & 1) << b;
                self.scb.shcsr
                    | act(EXC_MEMMANAGE, 0)
                    | act(EXC_BUSFAULT, 1)
                    | act(EXC_USAGEFAULT, 3)
                    | act(EXC_SVCALL, 7)
                    | act(EXC_DEBUGMON, 8)
                    | act(EXC_PENDSV, 10)
                    | act(EXC_SYSTICK, 11)
                    | pend(EXC_USAGEFAULT, 12)
                    | pend(EXC_MEMMANAGE, 13)
                    | pend(EXC_BUSFAULT, 14)
                    | pend(EXC_SVCALL, 15)
            }
            0xd28 => self.scb.cfsr,
            0xd2c => self.scb.hfsr,
            0xd30 => self.scb.dfsr,
            0xd34 => self.scb.mmfar,
            0xd38 => self.scb.bfar,
            0xd3c => self.scb.afsr,
            0xd88 => self.scb.cpacr,
            0xf00 => 0,
            _ => 0,
        })
    }

    pub(crate) fn ppb_write(&mut self, addr: u32, size: u32, value: u32) -> bool {
        // Sub-word writes: byte lanes of IPR / SHPR are real; other registers merge into the word.
        let off = addr & 0xfff;
        if addr & 0xfffff000 == 0xe000_e000 && size == 1 {
            if (0x400..0x4f0).contains(&off) {
                self.nvic.write_prio_byte(off, value as u8);
                return true;
            }
            if (0xd18..0xd24).contains(&off) {
                self.nvic.set_sys_prio((4 + off - 0xd18) as u16, value as u8);
                return true;
            }
        }
        let word = if size == 4 {
            value
        } else {
            let old = self.ppb_read_word(addr & !3).unwrap_or(0);
            let sh = (addr & 3) * 8;
            let m = if size == 2 { 0xffffu32 } else { 0xff } << sh;
            (old & !m) | ((value << sh) & m)
        };
        self.ppb_write_word(addr & !3, word);
        true
    }

    fn ppb_write_word(&mut self, addr: u32, v: u32) {
        let cycles = self.cpu.cycles;
        if addr & 0xfffff000 == 0xe000_1000 {
            match addr & 0xfff {
                0 => self.scb.dwt_ctrl = v & 1,
                4 => self.scb.dwt_off = cycles.wrapping_sub(v as u64),
                _ => {}
            }
            return;
        }
        if addr & 0xfffff000 != 0xe000_e000 {
            return;
        }
        let off = addr & 0xfff;
        match off {
            0x008 => self.scb.actlr = v,
            0x010 => self.systick.write_csr(v, cycles, &mut self.sched),
            0x014 => self.systick.write_rvr(v, cycles, &mut self.sched),
            0x018 => self.systick.write_cvr(cycles, &mut self.sched),
            0x100..=0x4ef | 0xf00 => self.nvic.write_word(off, v),
            0xd04 => {
                let n = &mut self.nvic;
                if v & 1 << 31 != 0 {
                    n.set_sys_pending(EXC_NMI);
                }
                if v & 1 << 28 != 0 {
                    n.set_sys_pending(EXC_PENDSV);
                }
                if v & 1 << 27 != 0 {
                    n.clear_sys_pending(EXC_PENDSV);
                }
                if v & 1 << 26 != 0 {
                    n.set_sys_pending(EXC_SYSTICK);
                }
                if v & 1 << 25 != 0 {
                    n.clear_sys_pending(EXC_SYSTICK);
                }
            }
            0xd08 => self.scb.vtor = v & 0xffff_ff80,
            0xd0c => {
                if v >> 16 == 0x05fa {
                    self.nvic.prigroup = ((v >> 8) & 7) as u8;
                    self.nvic.dirty = true;
                    if v & 4 != 0 {
                        self.reset_requested = true;
                    }
                }
            }
            0xd10 => self.scb.scr = v & 0x16,
            0xd14 => self.scb.ccr = (v & 0x31b) | CCR_STKALIGN,
            0xd18..=0xd23 => {
                for k in 0..4 {
                    self.nvic.set_sys_prio((4 + off - 0xd18 + k) as u16, (v >> (8 * k)) as u8);
                }
            }
            0xd24 => {
                self.scb.shcsr = v & (SHCSR_MEMFAULTENA | SHCSR_BUSFAULTENA | SHCSR_USGFAULTENA);
                let set = |m: &mut Machine, bit: u32, e: u16| {
                    if v & (1 << bit) != 0 {
                        m.nvic.set_sys_pending(e);
                    } else {
                        m.nvic.clear_sys_pending(e);
                    }
                };
                set(self, 15, EXC_SVCALL);
                self.nvic.dirty = true;
            }
            0xd28 => self.scb.cfsr &= !v,
            0xd2c => self.scb.hfsr &= !v,
            0xd30 => self.scb.dfsr &= !v,
            0xd34 => self.scb.mmfar = v,
            0xd38 => self.scb.bfar = v,
            0xd3c => self.scb.afsr = v,
            0xd88 => self.scb.cpacr = v,
            _ => {}
        }
    }
}
