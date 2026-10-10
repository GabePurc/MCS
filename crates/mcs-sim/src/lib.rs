//! Cycle-accurate AVR simulation engine for MCS.
//!
//! * [`avr::Machine`] — CPU executor + data bus + peripherals + pins for one device.
//! * [`target::Target`] — the architecture seam the session drives (implemented by `avr::Machine`).
//! * [`session::Session`] — debugger session: run control (real-time / max speed), stepping,
//!   breakpoints, state snapshots for the UI.

pub mod avr;
pub mod clock;
pub mod pins;
pub mod scheduler;
pub mod protocol;
pub mod session;
pub mod target;
