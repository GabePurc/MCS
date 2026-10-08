//! AVR simulation: CPU executor, machine (bus + pins + clock) and peripheral models.

pub mod cpu;
pub mod machine;
pub mod peripherals;

pub use cpu::{CallFrame, Cpu, StopReason};
pub use machine::{Cx, Event, Machine, Message, Peripheral, ResetSource, Sys, Trigger};
