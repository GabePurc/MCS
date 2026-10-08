//! Cycle-accurate AVR simulation engine for MCS.
//!
//! * [`avr::Machine`] — CPU executor + data bus + peripherals + pins for one device.
//! * [`session::Session`] — debugger session: run control (real-time / max speed), stepping,
//!   breakpoints, state snapshots for the UI.

pub mod avr;
pub mod clock;
pub mod pins;
pub mod scheduler;
pub mod protocol;
pub mod session;
