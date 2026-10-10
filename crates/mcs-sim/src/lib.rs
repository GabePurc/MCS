//! Cycle-accurate AVR simulation engine for MCS.
//!
//! * [`avr::Machine`] — CPU executor + data bus + peripherals + pins for one device.
//! * [`target::Target`] — the architecture seam the session drives (implemented by `avr::Machine`).
//! * [`arm::Machine`] — ARMv7-M (Cortex-M3/M4) core, NVIC, SysTick, memory bus and the STM32G4
//!   peripherals; implements [`target::Target`] like the AVR machine.
//! * [`session::Session`] — debugger session: run control (real-time / max speed), stepping,
//!   breakpoints, state snapshots for the UI.

pub mod arm;
pub mod avr;
pub mod clock;
pub mod pins;
pub mod scheduler;
pub mod protocol;
pub mod session;
pub mod target;
