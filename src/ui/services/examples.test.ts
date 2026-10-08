import { describe, expect, it } from 'vitest';
import { defIncludeName, templateFor } from './examples';

describe('templates', () => {
  it('names the avrasm2 include like Atmel', () => {
    expect(defIncludeName('ATtiny85')).toBe('tn85def.inc');
    expect(defIncludeName('ATmega328P')).toBe('m328Pdef.inc');
    expect(defIncludeName('ATtiny10')).toBe('tn10def.inc');
  });

  it('adapts the assembly template to the device', () => {
    const t = templateFor('asm', 'ATmega328P');
    expect(t).toContain('.include "m328Pdef.inc"');
    expect(t).not.toContain('tn10def');
  });
});
