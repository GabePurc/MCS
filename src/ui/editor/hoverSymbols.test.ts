import { describe, expect, it, vi } from 'vitest';
import type { SourceSymbol } from '../backend/types';
import { createSymbolCache, pickSymbol, symbolText } from './hoverSymbols';

const s = (name: string, scope: [number, number] | null, extra: Partial<SourceSymbol> = {}): SourceSymbol => ({
  name, kind: 'local', detail: name, value: null, line: 1, scope, doc: '', ...extra,
});

describe('pickSymbol', () => {
  const syms = [s('i', null, { kind: 'global' }), s('i', [10, 30]), s('i', [15, 20], { detail: 'inner' })];
  it('prefers the innermost scope, then file scope', () => {
    expect(pickSymbol(syms, 'i', 17, false)?.detail).toBe('inner');
    expect(pickSymbol(syms, 'i', 25, false)?.scope).toEqual([10, 30]);
    expect(pickSymbol(syms, 'i', 5, false)?.kind).toBe('global');
  });
  it('ignores out-of-scope locals', () => {
    expect(pickSymbol([s('x', [10, 12])], 'x', 20, false)).toBeUndefined();
  });
  it('is case-insensitive only on request', () => {
    expect(pickSymbol([s('Loop', null)], 'loop', 1, true)).toBeDefined();
    expect(pickSymbol([s('Loop', null)], 'loop', 1, false)).toBeUndefined();
  });
});

describe('symbolText', () => {
  it('shows kind, line and value', () => {
    expect(symbolText(s('N', null, { kind: 'constant', value: 8, line: 3 })).meta).toBe('constant — line 3 = 8 (0x08)');
    expect(symbolText(s('RAMEND', null, { kind: 'device constant', value: 0x25f, line: 0 })).meta).toBe('device constant = 607 (0x025F)');
    expect(symbolText(s('t', null, { kind: 'register alias', value: 16, line: 2 })).meta).toBe('register alias — line 2');
  });
});

describe('createSymbolCache', () => {
  it('reuses the result for unchanged text', async () => {
    const fetch = vi.fn(async () => [s('a', null)]);
    const get = createSymbolCache(fetch);
    await get('x', 'c', 'd');
    await get('x', 'c', 'd');
    expect(fetch).toHaveBeenCalledTimes(1);
    await get('y', 'c', 'd');
    expect(fetch).toHaveBeenCalledTimes(2);
  });
});
