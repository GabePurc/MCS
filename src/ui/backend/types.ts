/**
 * TypeScript mirrors of the JSON produced by the Rust backend (serde, camelCase).
 * Keep in sync with crates/mcs-core/src/{program.rs, avr/device.rs} and
 * crates/mcs-sim/src/protocol.rs.
 */

// ---------------------------------------------------------------- program.rs
export type Severity = 'error' | 'warning' | 'info';

export interface Diagnostic {
  severity: Severity;
  message: string;
  file: string;
  line: number;
  column: number;
}

export type SymbolKind = 'func' | 'object' | 'label' | 'const' | 'section' | 'other';
export type SymbolSpace = 'code' | 'data' | 'eeprom' | 'none';

export interface ProgramSymbol {
  name: string;
  /** Byte address within its space. */
  address: number;
  size: number;
  kind: SymbolKind;
  space: SymbolSpace;
  global: boolean;
}

export interface LineEntry {
  /** Byte address in program memory. */
  address: number;
  file: number;
  line: number;
  isStmt: boolean;
}

export interface LoadedProgram {
  format: 'asm' | 'hex' | 'elf' | 'mc';
  flash: number[];
  flashUsed: number;
  eeprom?: number[];
  fuses?: number[];
  lock?: number[];
  entry: number;
  symbols: ProgramSymbol[];
  files: string[];
  lines: LineEntry[];
  device?: string;
  diagnostics: Diagnostic[];
  /** Additional loadable segments (RISC-V: address + bytes). */
  segments?: { address: number; data: number[] }[];
}

// ---------------------------------------------------------------- device.rs
export interface BitFieldSpec {
  name: string;
  mask: number;
  desc?: string;
}

export interface IoRegisterSpec {
  name: string;
  /** Data-space address. */
  addr: number;
  reset: number;
  group: string;
  desc: string;
  bits: BitFieldSpec[];
  access: 'rw' | 'r' | 'w';
}

export interface VectorSpec {
  index: number;
  name: string;
  desc: string;
}

export interface PinSpec {
  number: number;
  name: string;
  kind: 'io' | 'vcc' | 'gnd' | 'ref';
  gpio?: number;
  functions: string[];
}

export interface FuseBitSpec {
  name: string;
  /** Contiguous mask; programmed = 0. */
  mask: number;
  desc: string;
}

export interface FuseByteSpec {
  name: string;
  default: number;
  bits: FuseBitSpec[];
}

export type SleepKind = 'idle' | 'adc-noise-reduction' | 'power-down' | 'power-save' | 'standby' | 'extended-standby';

export interface AvrDeviceSpec {
  arch: 'avr';
  id: string;
  name: string;
  family: string;
  coreName: string;
  features: number;
  flashSize: number;
  sramStart: number;
  sramSize: number;
  eepromSize: number;
  ioBase: number;
  ioSize: number;
  regsInDataSpace: boolean;
  flashMapBase: number | null;
  nvmMap: { lock: number; config: number; calibration: number; signature: number } | null;
  signature: [number, number, number];
  calibration: number;
  fuses: FuseByteSpec[];
  sleep: { register: string; seMask: number; smMask: number; modes: [number, SleepKind][] };
  boot: { sizesWords: [number, number, number, number] } | null;
  vectors: VectorSpec[];
  registers: IoRegisterSpec[];
  groups: { name: string; desc: string }[];
  package: string;
  pins: PinSpec[];
  gpioCount: number;
  hasAdc: boolean;
  clock: { internalHz: number; slowHz: number; defaultPrescaleLog2: number };
  vcc: number;
  vccRange: [number, number];
  /** [max clock Hz, minimum VCC] pairs. */
  speedGrades: [number, number][];
  datasheet: string;
  die: { widthUm: number; heightUm: number; photoUrl: string; photoCredit: string } | null;
  peripheralSet: string;
}

/** Memory-mapped register of an ARM device (mirrors mcs_core::arm::device::MmioRegisterSpec). */
export interface MmioRegisterSpec {
  name: string;
  /** Absolute bus address. */
  addr: number;
  /** Register width in bytes (1, 2 or 4). */
  size: number;
  reset: number;
  /** Peripheral instance the register belongs to. */
  group: string;
  desc: string;
  /** Bit-field masks are 32-bit (multi-bit fields use contiguous masks). */
  bits: BitFieldSpec[];
  access: 'rw' | 'r' | 'w';
}

export interface ArmVectorSpec {
  /** Exception number: 1 Reset ... 15 SysTick, 16 + n for external interrupt n. */
  index: number;
  name: string;
  desc: string;
}

export interface ArmDeviceSpec {
  arch: 'arm';
  id: string;
  name: string;
  family: string;
  coreName: string;
  /** ArmFeatures bits: 1 DSP, 2 FPv4-SP, 4 FPv5-D16. */
  features: number;
  cpuid: number;
  flashBase: number;
  flashSize: number;
  sramBase: number;
  /** Main SRAM in bytes; the CCM SRAM (when present) follows it in the data the session sends. */
  sramSize: number;
  ccmSram: { base: number; size: number; aliasBase: number } | null;
  /** Further RAM blocks (STM32H7: ITCM, AXI SRAM, SRAM1-3, SRAM4, backup SRAM); block k is `extraRam[k - 1]`. */
  extraRam: { name: string; base: number; size: number }[];
  registers: MmioRegisterSpec[];
  groups: { name: string; desc: string }[];
  vectors: ArmVectorSpec[];
  nirq: number;
  nvicPrioBits: number;
  package: string;
  pins: PinSpec[];
  /** Size of the GPIO array (ports * 16); pin index = port * 16 + bit, so there are gaps. */
  gpioCount: number;
  clock: { hsiHz: number; lsiHz: number; hseMinHz: number; hseMaxHz: number; hseDefaultHz: number };
  vcc: number;
  vccRange: [number, number];
  speedGrades: [number, number][];
  datasheet: string;
  die: { widthUm: number; heightUm: number; photoUrl: string; photoCredit: string } | null;
  peripheralSet: {
    rccBase: number;
    flashBase: number;
    pwrBase: number;
    syscfgBase: number;
    extiBase: number;
    gpio: { name: string; port: number; base: number }[];
    uarts: { name: string; kind: 'usart' | 'uart' | 'lpuart'; base: number; irq: number; apb: number }[];
    timers: { name: string; base: number; irq: number; width: number; channels: number; apb: number }[];
    extiIrqs: number[];
  };
}

// ---------------------------------------------------------------- protocol.rs
export type ExtDrive = 'float' | 'low' | 'high' | 'analog';
export type StepKind = 'into' | 'over' | 'out';
/** realtime: sim time = wall time x factor; clock: factor CPU cycles per second; max: unthrottled. */
export type SpeedMode = 'realtime' | 'clock' | 'max';

export interface PinGenerator {
  hz: number;
  /** Active fraction of each period (0..1). */
  duty: number;
  /** Number of pulses (burst), absent = continuous. */
  count?: number;
  /** Idle high, active low. */
  invert: boolean;
}

export type SimCommand =
  | { type: 'init'; deviceId: string }
  | { type: 'load'; deviceId: string; program: LoadedProgram }
  | { type: 'run' }
  | { type: 'pause' }
  | { type: 'reset' }
  | { type: 'powerCycle' }
  | { type: 'step'; kind: StepKind; source: boolean }
  | { type: 'runTo'; pc: number }
  | { type: 'setBreakpoints'; pcs: number[] }
  | { type: 'setSpeed'; mode: SpeedMode; factor: number }
  | { type: 'setPin'; pin: number; ext: ExtDrive; volts: number }
  | { type: 'setVcc'; volts: number }
  | { type: 'setExternalClock'; hz: number }
  | { type: 'setClockConfig'; source: number; prescaleLog2: number }
  | { type: 'setPinGenerator'; pin: number; gen: PinGenerator | null }
  | { type: 'setProfiling'; enabled: boolean }
  /** ARM / RISC-V: which extra RAM block (`extraRam[index - 1]`) the memory view watches; 0 = none. */
  | { type: 'watchRam'; index: number }
  | { type: 'writeData'; addr: number; value: number }
  /** Wide (1/2/4-byte) write through the bus; needed for ARM / RISC-V peripheral registers. */
  | { type: 'writeMem'; addr: number; size: number; value: number }
  | { type: 'writeFlash'; addr: number; value: number }
  | { type: 'writeReg'; reg: number; value: number }
  | { type: 'writeCpu'; field: CpuField; value: number }
  | { type: 'writeFuse'; index: number; value: number }
  | { type: 'writeEeprom'; addr: number; value: number }
  | { type: 'setSerial'; config: SerialConfig }
  | { type: 'serialSend'; bytes: number[] }
  | { type: 'requestState' };

/** `pc` is in the architecture's native unit; `sreg` is AVR-only, `mstatus`..`mscratch` RISC-V-only, the rest ARM-only. */
export type CpuField = 'pc' | 'sp' | 'sreg' | 'xpsr' | 'msp' | 'psp' | 'lr' | 'control' | 'primask' | 'basepri' | 'faultmask' | 'fpscr' | 'mstatus' | 'mie' | 'mtvec' | 'mepc' | 'mcause' | 'mtval' | 'mscratch';

export interface SerialConfig {
  /** GPIO decoded into the Serial Monitor (the MCU's TX). */
  monitor: number | null;
  /** GPIO driven with the bytes typed in the Serial Monitor (the MCU's RX). */
  inject: number | null;
  baud: number;
  dataBits: number;
  /** 0 none, 1 even, 2 odd. */
  parity: number;
  stopBits: number;
}

export interface PinState {
  level: number;
  dir: number;
  out: number;
  pullup: number;
  /** Pull-down enabled (RISC-V devices). */
  pulldown?: number;
  ovEnable: number;
  ext: ExtDrive;
  extVolts: number;
  volts: number;
  reserved: boolean;
  /** Function holding a reserved pin ("RESET", "XTAL1"...). */
  reservedBy?: string;
  gen?: PinGenerator;
}

export interface CallFrame {
  returnPc: number;
  targetPc: number;
  vector: number;
  sp: number;
}

export interface StopInfo {
  reason: 'breakpoint' | 'break' | 'invalid' | 'step' | 'pause' | 'runTo' | 'reset' | 'load';
  pc: number;
  message?: string;
}

export interface SimMessage {
  cycle: number;
  level: 'info' | 'warning' | 'error';
  text: string;
}

/** RISC-V (ESP32-C3) device spec (mirrors mcs_core::riscv::device::RiscvDeviceSpec). */
export interface RiscvDeviceSpec {
  arch: 'riscv';
  id: string;
  name: string;
  family: string;
  coreName: string;
  isa: string;
  flashBase: number;
  dromBase: number;
  flashSize: number;
  flashExternal: boolean;
  sramBase: number;
  sramSize: number;
  iramBase: number;
  extraRam: { name: string; base: number; size: number }[];
  memoryMap: { name: string; base: number; size: number; perm: string; desc: string }[];
  registers: MmioRegisterSpec[];
  groups: { name: string; desc: string }[];
  interrupts: { source: number; name: string; desc: string }[];
  cpuInterrupts: number;
  package: string;
  pins: PinSpec[];
  gpioCount: number;
  strapping: number[];
  clock: { xtalHz: number; rcFastHz: number; rcSlowHz: number; systimerHz: number; cpuMaxHz: number };
  vcc: number;
  vccRange: [number, number];
  speedGrades: [number, number][];
  datasheet: string;
  die: { widthUm: number; heightUm: number; photoUrl: string; photoCredit: string } | null;
  peripheralSet: unknown;
}

/** Any supported device spec; discriminated by `arch` (more architectures are added to the union). */
export type DeviceSpec = AvrDeviceSpec | ArmDeviceSpec | RiscvDeviceSpec;
export type Arch = DeviceSpec['arch'];

/** Architecture-specific CPU state as received from Rust. */
export type RawCoreState =
  | { arch: 'avr'; sp: number; sreg: number; regs: number[] }
  | {
      arch: 'arm';
      /** r0-r15 (r13 = active SP, r15 = PC). */
      r: number[];
      xpsr: number;
      msp: number;
      psp: number;
      control: number;
      primask: boolean;
      basepri: number;
      faultmask: boolean;
      /** S0-S31 as raw bits (absent without an FPU). */
      fpr?: number[];
      fpscr: number;
    }
  | {
      arch: 'riscv';
      /** x0-x31 (x0 is always 0). */
      x: number[];
      mstatus: number;
      mie: number;
      mip: number;
      mtvec: number;
      mepc: number;
      mcause: number;
      mtval: number;
      mscratch: number;
    };
/** CPU state as used by the UI (typed arrays). */
export type CoreState =
  | { arch: 'avr'; sp: number; sreg: number; regs: Uint8Array }
  | { arch: 'arm'; r: Uint32Array; xpsr: number; msp: number; psp: number; control: number; primask: boolean; basepri: number; faultmask: boolean; fpr: Uint32Array; fpscr: number }
  | { arch: 'riscv'; x: Uint32Array; mstatus: number; mie: number; mip: number; mtvec: number; mepc: number; mcause: number; mtval: number; mscratch: number };
export type AvrCore = Extract<CoreState, { arch: 'avr' }>;
export type ArmCore = Extract<CoreState, { arch: 'arm' }>;
export type RiscvCore = Extract<CoreState, { arch: 'riscv' }>;

/** Raw state as received from Rust. */
export interface RawMachineState {
  running: boolean;
  /** Program counter in the architecture's native unit (AVR: word address). */
  pc: number;
  /** Program counter as a byte address. */
  pcBytes: number;
  core: RawCoreState;
  cycles: number;
  instructions: number;
  timeSec: number;
  hz: number;
  extClockHz: number;
  sleeping: boolean;
  sleepMode: number;
  resetHeld: boolean;
  /** AVR: data space. ARM: the SRAM image (main SRAM then CCM). RISC-V: SRAM1 as seen on the data bus. Empty while unchanged (non-AVR). */
  data: number[];
  /** ARM / RISC-V: values of the memory-mapped registers, aligned to `spec.registers`. */
  io?: number[];
  /** ARM / RISC-V: bytes of the extra RAM block selected with `watchRam`; present on the first state after the selection and when they changed. */
  ramExtra?: { index: number; data: number[] };
  flash?: number[];
  flashVersion: number;
  fuses: number[];
  lock: number;
  /** EEPROM contents, present only when they changed. */
  eeprom?: number[];
  /** Bytes received by the Serial Monitor since the previous state. */
  serial?: number[];
  serialConfig: SerialConfig;
  pins: PinState[];
  vcc: number;
  callStack: CallFrame[];
  peripherals: { name: string; values: [string, string][] }[];
  speedHz: number;
  traceFrom: number;
  traceCycles: number[];
  /** 32-bit words per trace entry; `traceLevels` is flattened (pin i = bit i % 32 of word i / 32). */
  traceWords: number;
  traceLevels: number[];
  /** Instructions executed per word since the previous state (profiling only). */
  execHeat?: number[];
  messages: SimMessage[];
  stop?: StopInfo;
}

/** State as used by the UI (typed arrays). */
export interface MachineState extends Omit<RawMachineState, 'core' | 'data' | 'io' | 'flash' | 'traceCycles' | 'traceLevels' | 'execHeat' | 'eeprom' | 'ramExtra'> {
  eeprom?: Uint8Array;
  ramExtra?: { index: number; data: Uint8Array };
  core: CoreState;
  data: Uint8Array;
  io: Uint32Array;
  flash?: Uint8Array;
  traceCycles: Float64Array;
  traceLevels: Uint32Array;
  execHeat?: Uint32Array;
}

export const isAvr = (spec: DeviceSpec): spec is AvrDeviceSpec => spec.arch === 'avr';
export const isArm = (spec: DeviceSpec): spec is ArmDeviceSpec => spec.arch === 'arm';
export const isRiscv = (spec: DeviceSpec): spec is RiscvDeviceSpec => spec.arch === 'riscv';

/** AVR CPU state of a snapshot (throws for other architectures). */
export function avrCore(st: MachineState): AvrCore {
  if (st.core.arch !== 'avr') throw new Error(`Expected an AVR state, got ${st.core.arch}`);
  return st.core;
}

/** ARM CPU state of a snapshot (throws for other architectures). */
export function armCore(st: MachineState): ArmCore {
  if (st.core.arch !== 'arm') throw new Error(`Expected an ARM state, got ${st.core.arch}`);
  return st.core;
}

/** RISC-V CPU state of a snapshot (throws for other architectures). */
export function riscvCore(st: MachineState): RiscvCore {
  if (st.core.arch !== 'riscv') throw new Error(`Expected a RISC-V state, got ${st.core.arch}`);
  return st.core;
}

/** Bytes per unit of the architecture's native program counter (AVR: 16-bit words, ARM/RISC-V: bytes). */
export const pcUnit = (arch: Arch): number => (arch === 'avr' ? 2 : 1);
/** Native program counter -> byte address in the program's address space. */
export const pcToBytes = (arch: Arch, pc: number): number => pc * pcUnit(arch);
/** Byte address -> native program counter (rounded down). */
export const bytesToPc = (arch: Arch, bytes: number): number => Math.floor(bytes / pcUnit(arch));
/** Address of the first flash byte on the bus (0 on AVR). */
export const flashBaseOf = (spec: DeviceSpec): number => (spec.arch === 'avr' ? 0 : spec.flashBase);

/** Whether the FPU is present (FPv4-SP or FPv5). */
export const armHasFpu = (spec: ArmDeviceSpec): boolean => (spec.features & 6) !== 0;
/** Whether the FPU handles double precision (FPv5-D16). */
export const armHasDouble = (spec: ArmDeviceSpec): boolean => (spec.features & 4) !== 0;

/** Converts the raw core state of a snapshot (typed arrays). */
export function convertCore(c: RawCoreState): CoreState {
  if (c.arch === 'avr') return { ...c, regs: Uint8Array.from(c.regs) };
  if (c.arch === 'riscv') return { ...c, x: Uint32Array.from(c.x) };
  return { ...c, r: Uint32Array.from(c.r), fpr: Uint32Array.from(c.fpr ?? []) };
}

export type SimOutput =
  | { type: 'device'; spec: DeviceSpec }
  | { type: 'state'; state: RawMachineState }
  | { type: 'error'; message: string };

// ---------------------------------------------------------------- app commands (src-tauri/src/lib.rs)
export interface BuildOutcome {
  ok: boolean;
  program: LoadedProgram | null;
  diagnostics: Diagnostic[];
  output: string;
  listing: string | null;
  deviceId: string;
}

export interface DeviceSummary {
  arch: Arch;
  id: string;
  name: string;
  family: string;
  flashSize: number;
  sramSize: number;
  package: string;
  coreName: string;
}

export interface McHint {
  line: number;
  /** Byte address of the line's first word. */
  address: number;
  text: string;
  valid: boolean;
}

export interface McAnnotations {
  hints: McHint[];
  diagnostics: Diagnostic[];
}

export interface DisasmLine {
  pc: number;
  words: number;
  raw: number[];
  mnemonic: string;
  operands: string;
  target: number | null;
  valid: boolean;
}

export interface InsnInfo {
  mnemonic: string;
  operands: string;
  encoding: string;
  cycles: number;
  words: number;
  summary: string;
  operation: string;
  flags: string;
  aliases: string;
  /** Beginner help: what the instruction is for and how to use it. */
  usage: string;
  example: string;
  /** Canonical mnemonic when this row is an assembler alias ('' otherwise). */
  aliasOf: string;
}

export interface ToolchainInfo {
  gcc: string;
  version: string;
}

/** User-defined microcontroller (mirrors mcs_core::avr::devices::CustomMcuConfig). */
export interface CustomMcuConfig {
  id: string;
  name: string;
  flashSize: number;
  sramSize: number;
  eepromSize: number;
  ports: number;
  extInterrupts: number;
  timers8: number;
  timers16: number;
  usarts: number;
  spis: number;
  twis: number;
  adcChannels: number;
  analogComparator: boolean;
  hardwareMultiplier: boolean;
  package: 'DIP' | 'SOIC';
  internalHz: number;
  maxHz: number;
  vcc: number;
}

export interface CustomPreview {
  package: string;
  pins: number;
  gpios: number;
  registers: number;
  vectors: number;
  coreName: string;
  sramStart: number;
  ramEnd: number;
  groups: string[];
}

export interface CustomRegistration {
  arch: Arch;
  id: string;
  error: string | null;
}
