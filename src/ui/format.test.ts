import { describe, expect, it } from 'vitest';
import { formatHz, formatTime, hex, parseNumber } from './format';

describe('format', () => {
  it('parses numbers in the notations the debugger accepts', () => {
    expect(parseNumber('0x1F')).toBe(31);
    expect(parseNumber('$1f')).toBe(31);
    expect(parseNumber('0b101')).toBe(5);
    expect(parseNumber('42')).toBe(42);
    expect(parseNumber('1Fh')).toBe(31);
    expect(parseNumber('zz')).toBeNaN();
  });

  it('formats values', () => {
    expect(hex(10)).toBe('0x0A');
    expect(hex(0x5f, 4)).toBe('0x005F');
    expect(formatHz(1_000_000)).toBe('1 MHz');
    expect(formatHz(128_000)).toBe('128 kHz');
    expect(formatTime(0.05)).toBe('50.000 ms');
    expect(formatTime(2e-6)).toBe('2.00 µs');
  });
});
