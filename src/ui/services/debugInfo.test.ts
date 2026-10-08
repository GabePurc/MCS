import { describe, expect, it } from 'vitest';
import type { LoadedProgram } from '../backend/types';
import { SymbolIndex, pcToSource, sameFile, sourceToPc } from './debugInfo';

const program: LoadedProgram = {
  format: 'asm',
  flash: [],
  flashUsed: 8,
  entry: 0,
  files: ['/proj/main.asm'],
  lines: [
    { address: 0, file: 0, line: 10, isStmt: true },
    { address: 2, file: 0, line: 12, isStmt: true },
    { address: 6, file: 0, line: 15, isStmt: true },
  ],
  symbols: [
    { name: 'reset', address: 2, size: 0, kind: 'label', space: 'code', global: false },
    { name: 'counter', address: 0x40, size: 1, kind: 'label', space: 'data', global: false },
  ],
  diagnostics: [],
};

describe('debugInfo', () => {
  it('maps addresses to source lines and back', () => {
    expect(pcToSource(program, 1)).toEqual({ file: '/proj/main.asm', line: 12 });
    expect(pcToSource(program, 2)).toEqual({ file: '/proj/main.asm', line: 12 });
    expect(sourceToPc(program, 'main.asm', 12).pc).toBe(1);
    // A breakpoint on a line without code moves to the next line that has code.
    expect(sourceToPc(program, 'main.asm', 13)).toEqual({ pc: 3, line: 15 });
    expect(sourceToPc(program, 'other.asm', 12).pc).toBe(-1);
  });

  it('matches files by path or base name', () => {
    expect(sameFile('/a/b/main.asm', 'main.asm')).toBe(true);
    expect(sameFile('C:\\x\\main.asm', '/y/main.asm')).toBe(true);
    expect(sameFile('/a/main.asm', 'blink.asm')).toBe(false);
  });

  it('describes code addresses relative to symbols', () => {
    const idx = new SymbolIndex(program);
    expect(idx.describeCode(2)).toBe('reset');
    expect(idx.describeCode(6)).toBe('reset+0x4');
    expect(idx.dataLabel(0x40)).toBe('counter');
  });
});
