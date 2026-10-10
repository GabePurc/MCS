//! RV32IMC simulation core (the ESP32-C3's ESP-RISC-V hart), standalone: not yet wired into the
//! session / `Target` (stage E2).
//!
//! * [`cpu`] — registers, pc, counters and the machine-mode CSR file.
//! * [`bus`] — memory windows with permissions, MMIO dispatch ([`bus::Mmio`]) and the lazily
//!   filled pre-decoded instruction cache with write invalidation.
//! * [`machine`] — run loop, executor, traps, interrupts (`mip`/`mie`/`mtvec` direct + vectored),
//!   `wfi`, cycle approximation.
//!
//! Instruction decoding and disassembly live in [`mcs_core::riscv`].
//!
//! References: The RISC-V Instruction Set Manual Vol. I (Unprivileged) and Vol. II (Privileged),
//! 20240411; ESP32-C3 Technical Reference Manual (ESP-RISC-V CPU, System and Memory).

pub mod bus;
pub mod cpu;
pub mod machine;

pub use bus::{AccessFault, Bus, Cx, MemId, Mmio, PERM_R, PERM_RW, PERM_RWX, PERM_RX, PERM_W, PERM_X};
pub use cpu::{Cpu, HartIds};
pub use machine::{CsrHook, Machine, RvConfig, StopReason};
