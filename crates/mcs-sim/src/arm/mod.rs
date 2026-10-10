//! ARMv7-M (Cortex-M3/M4) simulation core.
//!
//! * [`cpu`] — registers, xPSR, stack pointer banking.
//! * [`bus`] — memory map (flash, SRAM, MMIO peripherals via [`bus::Mmio`], System Control Space).
//! * [`exec`] — instruction executor (dense `match` on the pre-decoded [`mcs_core::arm::thumb::Op`]).
//! * [`machine`] — run loop, exception entry/return, fault escalation.
//! * [`nvic`], [`scb`], [`systick`] — interrupt controller, system control block, SysTick.
//!
//! This module is standalone for now: the session / protocol layers do not use it yet.
//!
//! References: ARM DDI 0403E.e (ARMv7-M ARM), ARM DDI 0439B (Cortex-M4 TRM, cycle counts),
//! ARM DUI 0553 (Cortex-M4 Devices Generic User Guide, NVIC/SCB/SysTick registers).

pub mod bus;
pub mod cpu;
pub mod exec;
pub mod machine;
pub mod nvic;
pub mod scb;
pub mod systick;

pub use bus::{Bus, Cx, MemConfig, Mmio};
pub use cpu::{Cpu, StopReason};
pub use machine::{ArmConfig, Machine};
