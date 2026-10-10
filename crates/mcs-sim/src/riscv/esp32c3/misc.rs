//! Generic register storage for peripherals that only need to keep their values, and the catch-all device
//! for unmodelled peripheral address ranges.

use mcs_core::riscv::device::RiscvDeviceSpec;

use crate::riscv::bus::{Cx, Mmio};

/// A block of 32-bit registers that stores writes (read-only registers ignore them) and resets to the
/// values of the device description.
pub struct RegFile {
    pub regs: Vec<u32>,
    reset: Vec<u32>,
    ro: Vec<bool>,
}

impl RegFile {
    /// The registers of `group` in `spec`, `size` bytes from `base`.
    pub fn from_spec(spec: &RiscvDeviceSpec, group: &str, base: u32, size: u32) -> Self {
        let n = (size / 4) as usize;
        let mut reset = vec![0; n];
        let mut ro = vec![false; n];
        for r in spec.registers.iter().filter(|r| r.group == group) {
            let i = ((r.addr - base) / 4) as usize;
            if i < n {
                reset[i] = r.reset;
                ro[i] = r.access == mcs_core::avr::device::RegisterAccess::R;
            }
        }
        Self { regs: reset.clone(), reset, ro }
    }

    #[inline]
    pub fn get(&self, off: u32) -> u32 {
        self.regs.get((off / 4) as usize).copied().unwrap_or(0)
    }

    /// Stores `v` unless the register is read-only; returns the register index.
    #[inline]
    pub fn put(&mut self, off: u32, v: u32) {
        let i = (off / 4) as usize;
        if i < self.regs.len() && !self.ro[i] {
            self.regs[i] = v;
        }
    }

    /// Stores `v` even into read-only registers (hardware-driven values).
    pub fn force(&mut self, off: u32, v: u32) {
        if let Some(r) = self.regs.get_mut((off / 4) as usize) {
            *r = v;
        }
    }

    pub fn reset(&mut self) {
        self.regs.copy_from_slice(&self.reset);
    }
}

/// An address range without a model: reads return 0, writes are ignored, with a one-time warning per 4 KiB page.
pub struct Unimpl {
    pub base: u32,
}

impl Mmio for Unimpl {
    fn read(&mut self, off: u32, _size: u8, cx: &mut Cx) -> u32 {
        let c = cx.cycles;
        let a = self.base + off;
        cx.sys.warn_key(c, format!("unmapped-{:08x}", a & !0xfff), format!("Read of unimplemented peripheral register at 0x{a:08X} returns 0"));
        0
    }

    fn write(&mut self, off: u32, _size: u8, _value: u32, cx: &mut Cx) {
        let c = cx.cycles;
        let a = self.base + off;
        cx.sys.warn_key(c, format!("unmapped-{:08x}", a & !0xfff), format!("Write to unimplemented peripheral register at 0x{a:08X} ignored"));
    }

    fn peek(&mut self, _off: u32, _cx: &mut Cx) -> u32 {
        0
    }
}
