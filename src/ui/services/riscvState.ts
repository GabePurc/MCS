/** Helpers over the RISC-V core state shown by the Processor panel. */

/** ABI names of x0-x31 (x8 is also `fp`). */
export const RISCV_ABI: readonly string[] = [
  'zero', 'ra', 'sp', 'gp', 'tp', 't0', 't1', 't2', 's0', 's1', 'a0', 'a1', 'a2', 'a3', 'a4', 'a5', 'a6', 'a7',
  's2', 's3', 's4', 's5', 's6', 's7', 's8', 's9', 's10', 's11', 't3', 't4', 't5', 't6',
];

/** Short role of each register in the standard calling convention (tooltips). */
export const RISCV_ROLE: readonly string[] = [
  'hard-wired zero', 'return address', 'stack pointer', 'global pointer', 'thread pointer', 'temporary', 'temporary', 'temporary',
  'saved register / frame pointer', 'saved register', 'argument / return value', 'argument / return value',
  'argument', 'argument', 'argument', 'argument', 'argument', 'argument',
  'saved register', 'saved register', 'saved register', 'saved register', 'saved register', 'saved register', 'saved register', 'saved register', 'saved register', 'saved register',
  'temporary', 'temporary', 'temporary', 'temporary',
];

/** Name used for register `i` in the UI: `x8` is shown as `s0/fp`. */
export const riscvRegName = (i: number): string => (i === 8 ? 's0/fp' : RISCV_ABI[i]);

/** mstatus bits shown as flags: [name, bit, description]. */
export const MSTATUS_FLAGS: [string, number, string][] = [
  ['MIE', 3, 'Machine interrupt enable (global)'],
  ['MPIE', 7, 'Machine previous interrupt enable (restored into MIE by mret)'],
];

/** Privilege mode in mstatus.MPP (the ESP32-C3 core is machine mode only, MPP reads as 3). */
export const mstatusMpp = (mstatus: number): number => (mstatus >>> 11) & 3;
export const MPP_NAMES = ['U (user)', 'S (supervisor)', 'reserved', 'M (machine)'];

/** mtvec mode: direct (all traps at BASE) or vectored (interrupts at BASE + 4 * cause). */
export function mtvecInfo(mtvec: number): { base: number; mode: 'direct' | 'vectored' | 'reserved' } {
  const m = mtvec & 3;
  return { base: (mtvec & ~3) >>> 0, mode: m === 0 ? 'direct' : m === 1 ? 'vectored' : 'reserved' };
}

const EXCEPTIONS: Record<number, string> = {
  0: 'Instruction address misaligned',
  1: 'Instruction access fault',
  2: 'Illegal instruction',
  3: 'Breakpoint',
  4: 'Load address misaligned',
  5: 'Load access fault',
  6: 'Store/AMO address misaligned',
  7: 'Store/AMO access fault',
  8: 'Environment call from U-mode',
  9: 'Environment call from S-mode',
  11: 'Environment call from M-mode',
  12: 'Instruction page fault',
  13: 'Load page fault',
  15: 'Store/AMO page fault',
};

/** Standard machine interrupts; the ESP32-C3 interrupt controller drives lines 1-31 (the cause is the line number). */
const STANDARD_INTERRUPTS: Record<number, string> = { 3: 'machine software interrupt', 7: 'machine timer interrupt', 11: 'machine external interrupt' };

/** Decoded mcause: whether it is an interrupt, its code and a readable name ("" for the reset value 0 with no trap yet). */
export function decodeMcause(mcause: number): { interrupt: boolean; code: number; name: string } {
  const interrupt = (mcause >>> 31) === 1;
  const code = (mcause & 0x7fffffff) >>> 0;
  if (interrupt) return { interrupt, code, name: `Interrupt ${code}${STANDARD_INTERRUPTS[code] ? ` (${STANDARD_INTERRUPTS[code]})` : ''}` };
  return { interrupt, code, name: EXCEPTIONS[code] ?? `Exception ${code}` };
}

/** Line numbers (bits 1-31) set in an interrupt mask such as mie / mip. */
export function interruptLines(mask: number): number[] {
  const out: number[] = [];
  for (let i = 1; i < 32; i++) if ((mask >>> i) & 1) out.push(i);
  return out;
}

/** Compact "1-4, 7, 11-31" style list of numbers. */
export function rangeList(nums: number[]): string {
  const parts: string[] = [];
  for (let i = 0; i < nums.length; ) {
    let j = i;
    while (j + 1 < nums.length && nums[j + 1] === nums[j] + 1) j++;
    parts.push(j - i >= 2 ? `${nums[i]}-${nums[j]}` : nums.slice(i, j + 1).join(', '));
    i = j + 1;
  }
  return parts.join(', ');
}
