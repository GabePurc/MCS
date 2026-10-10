//! The architecture seam: [`Target`] is what a debugger [`Session`](crate::session::Session)
//! needs from a simulated machine, whatever its CPU architecture. A session owns a
//! `Box<dyn Target>` and calls it once per time slice / command / state publish; the
//! per-instruction hot path stays inside the concrete machine (e.g. `avr::Machine::run`).

use std::any::Any;

use mcs_core::device::DeviceRef;
use mcs_core::program::LoadedProgram;

use crate::avr::Machine;
use crate::pins::{ExtDrive, PinGenerator};
use crate::protocol::{CpuField, MachineState, StepKind};
use crate::avr::peripherals::serial::SerialConfig;

pub use crate::avr::StopReason;

/// What a source- or instruction-level step request turned into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepPlan {
    /// Exactly one instruction: call [`Target::step_one`].
    Single,
    /// The target armed a stop condition: run normally (slices stay pausable).
    Run,
    /// Nothing to do (the target already logged why); the session just republishes.
    Refused,
}

/// How much of the target's incremental data the UI already has.
#[derive(Clone, Copy, Debug)]
pub struct Sent {
    /// Pin-trace sequence number already delivered.
    pub trace: u64,
    /// EEPROM (or other nonvolatile data) version already delivered; `u64::MAX` = nothing.
    pub eeprom: u64,
}

fn unsupported<T>(what: &str) -> Result<T, String> {
    Err(format!("{what} is not supported by this architecture"))
}

pub trait Target: Send {
    /// Downcast access to the concrete machine (tests, architecture-specific tooling).
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn device(&self) -> DeviceRef;

    // ---- program / reset
    /// Loads a program image and power-cycles; `None` just power-cycles.
    fn load_program(&mut self, program: Option<&LoadedProgram>);
    /// Debugger reset (CPU + peripherals, memories kept).
    fn debugger_reset(&mut self);
    /// Power-on reset (also clears the pin trace and the scheduler).
    fn power_cycle(&mut self);
    /// Tells the target which program it runs (builds the source-line map used by stepping).
    fn set_source_map(&mut self, program: Option<&LoadedProgram>);

    // ---- time and position
    fn cycles(&self) -> u64;
    fn elapsed_seconds(&self) -> f64;
    /// Cycle count reached at simulated time `seconds` (honours clock changes).
    fn cycle_at(&self, seconds: f64) -> u64;
    /// Program counter in the architecture's native unit (AVR: word address).
    fn pc(&self) -> u32;

    // ---- execution
    /// Runs until `limit` cycles, a breakpoint, a stop condition or an error.
    fn run_until(&mut self, limit: u64) -> StopReason;
    /// Executes exactly one instruction.
    fn step_one(&mut self) -> StopReason;
    /// Arms a stop when `pc` (native unit) is reached.
    fn run_to(&mut self, pc: u32);
    /// Arms the stop condition for a step. `source` selects source-line granularity (when the
    /// program has line info) instead of instructions.
    fn begin_step(&mut self, kind: StepKind, source: bool) -> StepPlan;
    /// Removes any armed run-to / step condition.
    fn clear_stop_condition(&mut self);
    fn set_breakpoints(&mut self, pcs: &[u32]);

    // ---- environment
    fn set_pin_input(&mut self, pin: usize, ext: ExtDrive, volts: f64);
    fn set_pin_generator(&mut self, pin: usize, gen: Option<PinGenerator>);
    fn set_vcc(&mut self, volts: f64);
    fn set_external_clock(&mut self, hz: f64);
    fn set_profiling(&mut self, enabled: bool);
    /// Selects the extra RAM block the memory view watches (0 = none); a no-op where there are none.
    fn watch_ram(&mut self, index: usize) -> Result<(), String> {
        let _ = index;
        Ok(())
    }
    fn set_serial(&mut self, config: SerialConfig);
    fn serial_send(&mut self, bytes: &[u8]);

    // ---- debugger writes (architecture-specific ones default to an error)
    fn write_data(&mut self, addr: u32, value: u8) -> Result<(), String>;
    /// Writes `size` (1, 2 or 4) bytes through the CPU's bus; defaults to byte writes only.
    fn write_mem(&mut self, addr: u32, size: u8, value: u32) -> Result<(), String> {
        if size == 1 {
            self.write_data(addr, value as u8)
        } else {
            unsupported("Wide memory writes")
        }
    }
    fn write_flash(&mut self, addr: u32, value: u8) -> Result<(), String> {
        let _ = (addr, value);
        unsupported("Writing program memory")
    }
    fn write_reg(&mut self, reg: usize, value: u32) -> Result<(), String> {
        let _ = (reg, value);
        unsupported("Writing CPU registers")
    }
    fn write_cpu(&mut self, field: CpuField, value: u32) -> Result<(), String> {
        let _ = (field, value);
        unsupported("Writing CPU state")
    }
    fn write_eeprom(&mut self, addr: u32, value: u8) -> Result<(), String> {
        let _ = (addr, value);
        unsupported("Writing EEPROM")
    }
    /// Writes fuse byte `index` and power-cycles.
    fn write_fuse(&mut self, index: usize, value: u8) -> Result<(), String> {
        let _ = (index, value);
        unsupported("Writing fuses")
    }
    /// Debugger write of the clock source and prescaler.
    fn set_clock_config(&mut self, source: u8, prescale_log2: u8) -> Result<(), String> {
        let _ = (source, prescale_log2);
        unsupported("Changing the clock configuration")
    }

    // ---- state
    /// Builds a UI snapshot. `running`, `flash_version`, `speed_hz` and `stop` are filled in by
    /// the session; `include_flash` asks for the program memory image. Advances `sent`.
    fn snapshot(&mut self, sent: &mut Sent, include_flash: bool, max_trace: usize) -> MachineState;
}

/// Creates the simulation target for a registered device.
pub fn new_target(device: DeviceRef) -> Box<dyn Target> {
    match device {
        DeviceRef::Avr(spec) => Box::new(Machine::new(spec)),
        DeviceRef::Arm(spec) => Box::new(crate::arm::Machine::from_spec(spec)),
        DeviceRef::Riscv(spec) => Box::new(crate::riscv::Esp32c3::from_spec(spec)),
    }
}
