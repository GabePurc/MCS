import { describe, expect, it } from 'vitest';
import { hexGridCols } from './dieArt';

describe('hexGridCols', () => {
  it('stacks a tiny EEPROM in a narrow block instead of one squeezed row', () => {
    // 8 bytes in a tall, thin column: one byte per row reads best.
    expect(hexGridCols(8, 60, 400)).toBe(1);
  });
  it('keeps 16 columns for a wide block of 128 bytes', () => {
    expect(hexGridCols(128, 1600, 400)).toBe(16);
  });
  it('never uses more columns than bytes', () => {
    expect(hexGridCols(2, 1000, 10)).toBeLessThanOrEqual(2);
  });
});
