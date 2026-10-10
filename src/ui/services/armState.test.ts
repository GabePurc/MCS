import { describe, expect, it } from 'vitest';
import type { ArmDeviceSpec } from '../backend/types';
import { activeStack, exceptionName } from './armState';

const spec = { vectors: [{ index: 11, name: 'SVCall', desc: '' }, { index: 15, name: 'SysTick', desc: '' }, { index: 16, name: 'WWDG', desc: '' }] } as ArmDeviceSpec;

describe('arm state helpers', () => {
  it('names the active exception', () => {
    expect(exceptionName(spec, 0)).toBe('Thread mode');
    expect(exceptionName(spec, 15)).toBe('SysTick');
    expect(exceptionName(spec, 21)).toBe('IRQ5');
  });
  it('picks the active stack pointer', () => {
    expect(activeStack({ xpsr: 0x0100_0000, control: 0 })).toBe('MSP');
    expect(activeStack({ xpsr: 0x0100_0000, control: 2 })).toBe('PSP');
    // Handler mode always runs on MSP, whatever CONTROL.SPSEL says.
    expect(activeStack({ xpsr: 0x0100_000f, control: 2 })).toBe('MSP');
  });
});
