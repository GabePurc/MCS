import { describe, expect, it } from 'vitest';
import { armCore, armHasDouble, armHasFpu, avrCore, bytesToPc, convertCore, flashBaseOf, isArm, isAvr, isRiscv, pcToBytes, pcUnit, riscvCore, type ArmDeviceSpec, type AvrDeviceSpec, type MachineState, type RiscvDeviceSpec } from './types';

const armSpec = { arch: 'arm', flashBase: 0x08000000, features: 3 } as ArmDeviceSpec;
const m7Spec = { arch: 'arm', flashBase: 0x08000000, features: 7 } as ArmDeviceSpec;
const avrSpec = { arch: 'avr' } as AvrDeviceSpec;

describe('pc units', () => {
  it('AVR counts words, ARM counts bytes', () => {
    expect(pcUnit('avr')).toBe(2);
    expect(pcUnit('arm')).toBe(1);
    expect(pcToBytes('avr', 0x40)).toBe(0x80);
    expect(bytesToPc('avr', 0x81)).toBe(0x40);
    expect(pcToBytes('arm', 0x0800_00e8)).toBe(0x0800_00e8);
    expect(bytesToPc('arm', 0x0800_00e8)).toBe(0x0800_00e8);
  });

  it('stays exact for addresses above 2^31', () => {
    expect(bytesToPc('arm', 0xffff_fffe)).toBe(0xffff_fffe);
    expect(pcToBytes('arm', 0xe000_0000)).toBe(0xe000_0000);
  });

  it('flash base', () => {
    expect(flashBaseOf(armSpec)).toBe(0x0800_0000);
    expect(flashBaseOf(avrSpec)).toBe(0);
  });
});

describe('device spec helpers', () => {
  it('discriminates by arch', () => {
    expect(isArm(armSpec) && !isAvr(armSpec)).toBe(true);
    expect(isAvr(avrSpec) && !isArm(avrSpec)).toBe(true);
  });
  it('knows the FPU flavour', () => {
    expect(armHasFpu(armSpec)).toBe(true);
    expect(armHasDouble(armSpec)).toBe(false);
    expect(armHasDouble(m7Spec)).toBe(true);
    expect(armHasFpu({ ...armSpec, features: 1 })).toBe(false);
  });
});

describe('core state conversion', () => {
  it('converts the ARM core to typed arrays and tolerates a missing FPU', () => {
    const c = convertCore({ arch: 'arm', r: Array.from({ length: 16 }, (_, i) => 0x8000_0000 + i), xpsr: 0x6100_0000, msp: 0x2002_0000, psp: 0, control: 2, primask: false, basepri: 0, faultmask: false, fpscr: 0 });
    expect(c.arch).toBe('arm');
    if (c.arch !== 'arm') return;
    expect(c.r).toBeInstanceOf(Uint32Array);
    expect(c.r[15]).toBe(0x8000_000f);
    expect(c.fpr.length).toBe(0);
    const f = convertCore({ arch: 'arm', r: new Array(16).fill(0), xpsr: 0, msp: 0, psp: 0, control: 0, primask: false, basepri: 0, faultmask: false, fpr: [0x3f800000, 0xffffffff], fpscr: 0x03000000 });
    expect(f.arch === 'arm' && f.fpr[1]).toBe(0xffff_ffff);
  });

  it('converts the AVR core', () => {
    const c = convertCore({ arch: 'avr', sp: 0x25f, sreg: 0x82, regs: [1, 2, 3] });
    expect(c.arch === 'avr' && c.regs).toBeInstanceOf(Uint8Array);
  });

  it('accessors reject the other architecture', () => {
    const st = { core: convertCore({ arch: 'avr', sp: 0, sreg: 0, regs: [] }) } as MachineState;
    expect(() => armCore(st)).toThrow(/ARM/);
    expect(avrCore(st).sp).toBe(0);
  });
});

describe('RISC-V', () => {
  const rvSpec = { arch: 'riscv', flashBase: 0x4200_0000 } as RiscvDeviceSpec;
  const raw = { arch: 'riscv' as const, x: Array.from({ length: 32 }, (_, i) => 0x8000_0000 + i), mstatus: 8, mie: 0, mip: 0, mtvec: 0x4200_0001, mepc: 0, mcause: 0, mtval: 0, mscratch: 0 };

  it('counts pc in bytes and has a flash base', () => {
    expect(pcUnit('riscv')).toBe(1);
    expect(pcToBytes('riscv', 0x4200_0010)).toBe(0x4200_0010);
    expect(bytesToPc('riscv', 0x4200_0010)).toBe(0x4200_0010);
    expect(flashBaseOf(rvSpec)).toBe(0x4200_0000);
  });

  it('discriminates by arch', () => {
    expect(isRiscv(rvSpec) && !isAvr(rvSpec) && !isArm(rvSpec)).toBe(true);
    expect(isRiscv(avrSpec) || isRiscv(armSpec)).toBe(false);
  });

  it('converts the core and the accessor rejects other architectures', () => {
    const st = { core: convertCore(raw) } as MachineState;
    expect(riscvCore(st).x).toBeInstanceOf(Uint32Array);
    expect(riscvCore(st).x[31]).toBe(0x8000_001f);
    expect(() => armCore(st)).toThrow(/riscv/);
    expect(() => avrCore(st)).toThrow(/riscv/);
    const avr = { core: convertCore({ arch: 'avr', sp: 0, sreg: 0, regs: [] }) } as MachineState;
    expect(() => riscvCore(avr)).toThrow(/RISC-V/);
  });
});
