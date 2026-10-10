/** Helpers over the ARM core state shown by the Processor panel. */
import type { ArmCore, ArmDeviceSpec } from '../backend/types';

/** Name of the active exception (IPSR) from the vector table of the device. */
export function exceptionName(spec: ArmDeviceSpec, ipsr: number): string {
  if (ipsr === 0) return 'Thread mode';
  return spec.vectors.find((v) => v.index === ipsr)?.name ?? `IRQ${ipsr - 16}`;
}

/** Which stack pointer is active: handler mode always uses MSP, thread mode follows CONTROL.SPSEL. */
export function activeStack(c: Pick<ArmCore, 'xpsr' | 'control'>): 'MSP' | 'PSP' {
  return (c.xpsr & 0x1ff) === 0 && c.control & 2 ? 'PSP' : 'MSP';
}

