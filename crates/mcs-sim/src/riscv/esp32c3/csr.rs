//! Custom and optional CSRs of the ESP32-C3 hart that the generic core does not implement, provided through
//! [`CsrHook`] (ESP32-C3 TRM chapter "ESP-RISC-V CPU": CSR list; the ESP-IDF startup code touches several of
//! them).
//!
//! * `pmpcfg0-3` (0x3a0-0x3a3), `pmpaddr0-15` (0x3b0-0x3bf): stored, the PMP is not enforced.
//! * Trigger module `tselect` .. `mcontext` (0x7a0-0x7a8): read as 0, writes ignored (no triggers).
//! * Performance counters `mpcer` / `mpcmr` / `mpccr` (0x7e0-0x7e2): read as 0, writes ignored.
//! * Dedicated GPIO CSRs `cpu_gpio_oen` / `cpu_gpio_in` / `cpu_gpio_out` (0x800-0x802, assumption about their
//!   numbers): stored, not connected to the pads.

use crate::riscv::machine::CsrHook;

#[derive(Default)]
pub struct EspCsrs {
    pmpcfg: [u32; 4],
    pmpaddr: [u32; 16],
    dgpio: [u32; 3],
}

impl CsrHook for EspCsrs {
    fn read(&mut self, csr: u16, _cycles: u64) -> Option<u32> {
        Some(match csr {
            0x3a0..=0x3a3 => self.pmpcfg[(csr - 0x3a0) as usize],
            0x3b0..=0x3bf => self.pmpaddr[(csr - 0x3b0) as usize],
            0x7a0..=0x7a8 | 0x7e0..=0x7e2 => 0,
            0x800..=0x802 => self.dgpio[(csr - 0x800) as usize],
            _ => return None,
        })
    }

    fn write(&mut self, csr: u16, value: u32) -> bool {
        match csr {
            0x3a0..=0x3a3 => self.pmpcfg[(csr - 0x3a0) as usize] = value,
            0x3b0..=0x3bf => self.pmpaddr[(csr - 0x3b0) as usize] = value,
            0x7a0..=0x7a8 | 0x7e0..=0x7e2 => {}
            0x800..=0x802 => self.dgpio[(csr - 0x800) as usize] = value,
            _ => return false,
        }
        true
    }
}
