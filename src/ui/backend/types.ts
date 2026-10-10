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
  | { type: 'writeData'; addr: number; value: number }
  | { type: 'writeFlash'; addr: number; value: number }
  | { type: 'writeReg'; reg: number; value: number }
  | { type: 'writeCpu'; field: 'pc' | 'sp' | 'sreg'; value: number }
  | { type: 'writeFuse'; index: number; value: number }
  | { type: 'writeEeprom'; addr: number; value: number }
  | { type: 'setSerial'; config: SerialConfig }
  | { type: 'serialSend'; bytes: number[] }
  | { type: 'requestState' };

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

/** Raw state as received from Rust. */
export interface RawMachineState {
  running: boolean;
  pc: number;
  sp: number;
  sreg: number;
  cycles: number;
  instructions: number;
  timeSec: number;
  hz: number;
  extClockHz: number;
  sleeping: boolean;
  sleepMode: number;
  resetHeld: boolean;
  regs: number[];
  data: number[];
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
  traceLevels: number[];
  /** Instructions executed per word since the previous state (profiling only). */
  execHeat?: number[];
  messages: SimMessage[];
  stop?: StopInfo;
}

/** State as used by the UI (typed arrays). */
export interface MachineState extends Omit<RawMachineState, 'regs' | 'data' | 'flash' | 'traceCycles' | 'traceLevels' | 'execHeat' | 'eeprom'> {
  eeprom?: Uint8Array;
  regs: Uint8Array;
  data: Uint8Array;
  flash?: Uint8Array;
  traceCycles: Float64Array;
  traceLevels: Uint32Array;
  execHeat?: Uint32Array;
}

export type SimOutput =
  | { type: 'device'; spec: AvrDeviceSpec }
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
