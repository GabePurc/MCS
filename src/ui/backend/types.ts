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
  format: 'asm' | 'hex' | 'elf';
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
  kind: 'io' | 'vcc' | 'gnd';
  gpio?: number;
  functions: string[];
}

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
  fuseBits: { name: string; mask: number; desc: string }[];
  fuseDefault: number;
  vectors: VectorSpec[];
  registers: IoRegisterSpec[];
  groups: { name: string; desc: string }[];
  package: string;
  pins: PinSpec[];
  gpioCount: number;
  gpioPortName: string;
  hasAdc: boolean;
  clock: { internalHz: number; slowHz: number; defaultPrescaleLog2: number };
  vcc: number;
  peripheralSet: string;
}

// ---------------------------------------------------------------- protocol.rs
export type ExtDrive = 'float' | 'low' | 'high' | 'analog';
export type StepKind = 'into' | 'over' | 'out';
export type SpeedMode = 'realtime' | 'max';

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
  | { type: 'writeData'; addr: number; value: number }
  | { type: 'writeFlash'; addr: number; value: number }
  | { type: 'writeReg'; reg: number; value: number }
  | { type: 'writeCpu'; field: 'pc' | 'sp' | 'sreg'; value: number }
  | { type: 'writeFuse'; value: number }
  | { type: 'requestState' };

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
  sleeping: boolean;
  sleepMode: number;
  resetHeld: boolean;
  regs: number[];
  data: number[];
  flash?: number[];
  flashVersion: number;
  fuse: number;
  lock: number;
  pins: PinState[];
  vcc: number;
  callStack: CallFrame[];
  peripherals: { name: string; values: [string, string][] }[];
  speedHz: number;
  traceFrom: number;
  traceCycles: number[];
  traceLevels: number[];
  messages: SimMessage[];
  stop?: StopInfo;
}

/** State as used by the UI (typed arrays). */
export interface MachineState extends Omit<RawMachineState, 'regs' | 'data' | 'flash' | 'traceCycles' | 'traceLevels'> {
  regs: Uint8Array;
  data: Uint8Array;
  flash?: Uint8Array;
  traceCycles: Float64Array;
  traceLevels: Uint32Array;
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
}

export interface ToolchainInfo {
  gcc: string;
  version: string;
}
