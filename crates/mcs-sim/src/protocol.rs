//! Messages between the UI and the simulation session (serialized as camelCase JSON).

use mcs_core::avr::device::AvrDeviceSpec;
use mcs_core::program::LoadedProgram;
use serde::{Deserialize, Serialize};

use crate::avr::{CallFrame, Message};
use crate::pins::{ExtDrive, PinGenerator};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepKind {
    Into,
    Over,
    Out,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpeedMode {
    /// Simulated time = wall-clock time x factor.
    Realtime,
    /// Fixed rate of `factor` CPU cycles per wall-clock second, whatever the MCU clock is.
    Clock,
    /// As fast as the host can go.
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CpuField {
    Pc,
    Sp,
    Sreg,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Command {
    Init { device_id: String },
    Load { device_id: String, program: Box<LoadedProgram> },
    Run,
    Pause,
    Reset,
    PowerCycle,
    /// `source` selects source-line granularity (when line info exists) vs. instructions.
    Step { kind: StepKind, source: bool },
    /// Runs until the given word address is reached.
    RunTo { pc: u32 },
    SetBreakpoints { pcs: Vec<u32> },
    SetSpeed { mode: SpeedMode, factor: f64 },
    SetPin { pin: usize, ext: ExtDrive, volts: f64 },
    SetVcc { volts: f64 },
    SetExternalClock { hz: f64 },
    /// Debugger write of the clock source and prescaler (CLKMSR/CLKPSR, CCP handled).
    SetClockConfig { source: u8, prescale_log2: u8 },
    /// Attaches or removes a signal generator on a GPIO pin.
    SetPinGenerator { pin: usize, gen: Option<PinGenerator> },
    /// Per-word execution counting for the chip view's heat map.
    SetProfiling { enabled: bool },
    WriteData { addr: u16, value: u8 },
    WriteFlash { addr: u32, value: u8 },
    WriteReg { reg: usize, value: u8 },
    WriteCpu { field: CpuField, value: u32 },
    WriteFuse { value: u8 },
    RequestState,
    Shutdown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinState {
    pub level: u8,
    pub dir: u8,
    pub out: u8,
    pub pullup: u8,
    pub ov_enable: u8,
    pub ext: ExtDrive,
    pub ext_volts: f64,
    pub volts: f64,
    pub reserved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gen: Option<PinGenerator>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StopKind {
    Breakpoint,
    Break,
    Invalid,
    Step,
    Pause,
    RunTo,
    Reset,
    Load,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopInfo {
    pub reason: StopKind,
    /// Word address.
    pub pc: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeripheralInfo {
    pub name: String,
    pub values: Vec<(String, String)>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineState {
    pub running: bool,
    pub pc: u32,
    pub sp: u16,
    pub sreg: u8,
    pub cycles: u64,
    pub instructions: u64,
    pub time_sec: f64,
    pub hz: f64,
    /// Frequency assumed for the external clock input.
    pub ext_clock_hz: f64,
    pub sleeping: bool,
    pub sleep_mode: u8,
    pub reset_held: bool,
    pub regs: Vec<u8>,
    /// Data space (I/O + SRAM) as seen by the CPU, including live peripheral register values.
    pub data: Vec<u8>,
    /// Present only when program memory changed since the last state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flash: Option<Vec<u8>>,
    pub flash_version: u64,
    pub fuse: u8,
    pub lock: u8,
    pub pins: Vec<PinState>,
    pub vcc: f64,
    pub call_stack: Vec<CallFrame>,
    pub peripherals: Vec<PeripheralInfo>,
    /// Effective simulation speed (simulated cycles per wall-clock second).
    pub speed_hz: f64,
    pub trace_from: u64,
    pub trace_cycles: Vec<u64>,
    pub trace_levels: Vec<u32>,
    /// Instructions executed per word address since the previous state (profiling only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exec_heat: Vec<u32>,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<StopInfo>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Output {
    Device { spec: Box<AvrDeviceSpec> },
    State { state: Box<MachineState> },
    Error { message: String },
}
