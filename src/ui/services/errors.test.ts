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
    expect(shouldReport('ResizeObserver loop completed with undelivered notifications.')).toBe(false);
    expect(shouldReport('Internal error in Memory: A')).toBe(true);
    expect(shouldReport('Internal error in Memory: A')).toBe(true);
    expect(shouldReport('Internal error in Memory: A')).toBe(true);
    expect(shouldReport('Internal error in Memory: A')).toBe(false);
    expect(shouldReport('Internal error in Memory: B')).toBe(true);
  });
});
