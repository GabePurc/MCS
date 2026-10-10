//! Cycle-accurate AVR simulation engine for MCS.
//!
//! * [`avr::Machine`] — CPU executor + data bus + peripherals + pins for one device.
//! * [`arm::Machine`] — ARMv7-M (Cortex-M3/M4) core, NVIC, SysTick and memory bus (standalone, not
//!   yet wired into the session).
//! * [`session::Session`] — debugger session: run control (real-time / max speed), stepping,
//!   breakpoints, state snapshots for the UI.

pub mod arm;
pub mod avr;
pub mod clock;
pub mod pins;
pub mod scheduler;
pub mod protocol;
pub mod session;
