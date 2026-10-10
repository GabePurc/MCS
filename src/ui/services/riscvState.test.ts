import { describe, expect, it } from 'vitest';
import { decodeMcause, interruptLines, mstatusMpp, mtvecInfo, rangeList, riscvRegName, RISCV_ABI } from './riscvState';

describe('RISC-V helpers', () => {
  it('names the 32 registers by their ABI role', () => {
    expect(RISCV_ABI).toHaveLength(32);
    expect([0, 1, 2, 10, 17, 18, 27, 28, 31].map((i) => RISCV_ABI[i])).toEqual(['zero', 'ra', 'sp', 'a0', 'a7', 's2', 's11', 't3', 't6']);
    expect(riscvRegName(8)).toBe('s0/fp');
    expect(riscvRegName(9)).toBe('s1');
  });

  it('decodes mcause into interrupt or exception names', () => {
    expect(decodeMcause(2)).toEqual({ interrupt: false, code: 2, name: 'Illegal instruction' });
    expect(decodeMcause(11).name).toBe('Environment call from M-mode');
    expect(decodeMcause(0x8000000b)).toEqual({ interrupt: true, code: 11, name: 'Interrupt 11 (machine external interrupt)' });
    expect(decodeMcause(0x8000000c).name).toBe('Interrupt 12');
    expect(decodeMcause(0x4c).name).toBe('Exception 76');
  });

  it('splits mtvec into base and mode', () => {
    expect(mtvecInfo(0x42000101)).toEqual({ base: 0x42000100, mode: 'vectored' });
    expect(mtvecInfo(0x42000100)).toEqual({ base: 0x42000100, mode: 'direct' });
    expect(mtvecInfo(0xfffffffe).mode).toBe('reserved');
    expect(mtvecInfo(0xfffffffd).base).toBe(0xfffffffc);
  });

  it('reads mstatus.MPP and interrupt masks', () => {
    expect(mstatusMpp(0x1888)).toBe(3);
    expect(interruptLines(0b1000_1010)).toEqual([1, 3, 7]);
    expect(interruptLines(0xffffffff)).toHaveLength(31);
    expect(rangeList([1, 2, 3, 4, 7, 11, 12])).toBe('1-4, 7, 11, 12');
    expect(rangeList([])).toBe('');
  });
});
