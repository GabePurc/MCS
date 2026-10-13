import { describe, expect, it } from 'vitest';
import { describeError, shouldReport } from './errors';

describe('error reports', () => {
  it('describes thrown values', () => {
    expect(describeError(new TypeError('x is null'))).toBe('TypeError: x is null');
    expect(describeError('plain')).toBe('plain');
    expect(describeError({ code: 3 })).toBe('{"code":3}');
    expect(describeError(undefined)).toBe('undefined');
  });

  it('skips browser noise and floods of the same error', () => {
    expect(shouldReport('ResizeObserver loop completed with undelivered notifications.', 0)).toBe(false);
    // Alternating errors are limited per message, not just consecutive repeats.
    const shown = [];
    for (let i = 0; i < 8; i++) shown.push(shouldReport(i % 2 ? 'Internal error in A' : 'Internal error in B', 1000 + i));
    expect(shown).toEqual([true, true, true, true, true, true, false, false]);
    // The limit lifts after the flood window.
    expect(shouldReport('Internal error in A', 1001 + 10_000)).toBe(true);
  });
});
