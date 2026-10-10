//! Messages between the UI and the simulation session (serialized as camelCase JSON).

use mcs_core::device::DeviceRef;
use mcs_core::program::LoadedProgram;
use serde::{Deserialize, Serialize};

use crate::avr::{CallFrame, Message};
use crate::avr::peripherals::serial::SerialConfig;
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
    /// Program counter in the architecture's native unit (AVR: word address, ARM: byte address).
    Pc,
    /// AVR stack pointer / ARM active stack pointer.
    Sp,
    Sreg,
    /// ARM: xPSR flags (N, Z, C, V, Q).
    Xpsr,
    /// ARM: main / process stack pointer.
    Msp,
    Psp,
    /// ARM: link register (r14).
    Lr,
    /// ARM: CONTROL (nPRIV, SPSEL, FPCA).
    Control,
    Primask,
    Basepri,
    Faultmask,
    /// ARM: floating-point status and control register (FPU devices).
    Fpscr,
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
    /// Selects the extra RAM block (`extra_ram[index - 1]`, ARM) the memory view watches; 0 = none.
    /// The next state carries its bytes in `ram_extra`, later ones only when they changed.
    WatchRam { index: usize },
    /// Serial Monitor line settings.
    SetSerial { config: SerialConfig },
    /// Bytes typed in the Serial Monitor (sent into the injection pin).
    SerialSend { bytes: Vec<u8> },
    /// Debugger edit of the EEPROM.
    WriteEeprom { addr: u32, value: u8 },
    WriteData { addr: u32, value: u8 },
    /// Debugger write of `size` (1, 2 or 4) bytes through the CPU's bus (ARM peripheral registers
    /// need full-width accesses).
    WriteMem { addr: u32, size: u8, value: u32 },
    WriteFlash { addr: u32, value: u8 },
    WriteReg { reg: usize, value: u32 },
    WriteCpu { field: CpuField, value: u32 },
    /// Writes fuse byte `index` (0 = low / the configuration byte) and power-cycles.
    WriteFuse {
        #[serde(default)]
        index: usize,
        value: u8,
    },
    RequestState,
    Shutdown,
}

/// Bytes of one extra RAM block (`extra_ram[index - 1]`).
#[derive(Clone, Debug, Serialize)]
pub struct RamExtra {
    pub index: usize,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinState {
    pub level: u8,
    pub dir: u8,
    pub out: u8,
    pub pullup: u8,
    pub pulldown: u8,
    pub ov_enable: u8,
    pub ext: ExtDrive,
    pub ext_volts: f64,
    pub volts: f64,
    pub reserved: bool,
    /// Function holding the pin ("RESET", "XTAL1"...), empty when not reserved.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub reserved_by: &'static str,
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
    /// Program counter in the architecture's native unit (AVR: word address).
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

/// Architecture-specific CPU state (tagged by `arch` in JSON).
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "arch", rename_all = "lowercase")]
pub enum CoreState {
    Avr { sp: u16, sreg: u8, regs: Vec<u8> },
    /// ARMv7-M: r0-r15 (r13 = active SP, r15 = PC), xPSR, banked stack pointers, special registers;
    /// `fpr` (S0-S31 as raw bits) and `fpscr` are filled on devices with an FPU (`fpr` is empty
    /// otherwise).
    #[serde(rename_all = "camelCase")]
    Arm {
        r: [u32; 16],
        xpsr: u32,
        msp: u32,
        psp: u32,
        control: u8,
        primask: bool,
        basepri: u8,
        faultmask: bool,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        fpr: Vec<u32>,
        fpscr: u32,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineState {
    pub running: bool,
    /// Program counter in the architecture's native unit (AVR: word address).
    pub pc: u32,
    /// Program counter as a byte address.
    pub pc_bytes: u64,
    pub core: CoreState,
    pub cycles: u64,
    pub instructions: u64,
    pub time_sec: f64,
    pub hz: f64,
    /// Frequency assumed for the external clock input.
    pub ext_clock_hz: f64,
    pub sleeping: bool,
    /// AVR sleep mode (0 on other architectures).
    pub sleep_mode: u8,
    pub reset_held: bool,
    /// Data space (I/O + SRAM) as seen by the CPU, including live peripheral register values.
    ///
    /// ARM: the SRAM image (main SRAM followed by the CCM SRAM, starting at `sramBase`); empty
    /// when it did not change since the previous state.
    pub data: Vec<u8>,
    /// ARM: contents of the extra RAM block selected with `WatchRam`; present only on the first
    /// state after the selection (or a load) and when the bytes changed since.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram_extra: Option<RamExtra>,
    /// ARM: values of the memory-mapped peripheral and core registers, aligned to the device's
    /// `registers` list (empty on AVR).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub io: Vec<u32>,
    /// Present only when program memory changed since the last state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flash: Option<Vec<u8>>,
    pub flash_version: u64,
    /// Fuse bytes and lock bits (AVR; empty / 0 on architectures without them).
    pub fuses: Vec<u8>,
    pub lock: u8,
    /// EEPROM contents, present only when they changed since the last state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eeprom: Option<Vec<u8>>,
    /// Bytes received by the Serial Monitor since the last state.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub serial: Vec<u8>,
    pub serial_config: SerialConfig,
    pub pins: Vec<PinState>,
    pub vcc: f64,
    pub call_stack: Vec<CallFrame>,
    pub peripherals: Vec<PeripheralInfo>,
    /// Effective simulation speed (simulated cycles per wall-clock second).
    pub speed_hz: f64,
    pub trace_from: u64,
    pub trace_cycles: Vec<u64>,
    /// 32-bit words per trace entry (`ceil(GPIOs / 32)`); `trace_levels` holds them flattened.
    pub trace_words: u32,
    pub trace_levels: Vec<u32>,
    /// Instructions executed since the previous state as `[word, count, ...]` pairs for the
    /// words that ran (profiling only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exec_heat: Vec<u32>,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<StopInfo>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Output {
    Device { spec: DeviceRef },
    State { state: Box<MachineState> },
    Error { message: String },
}
