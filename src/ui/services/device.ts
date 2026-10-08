import { useSettings } from '../state/settings';
import { useWorkspace, appendOutput } from '../state/workspace';
import { sim } from './simClient';
import { loadProgram } from './build';
import type { AvrDeviceSpec } from '../backend/types';

/** GPIO names by GPIO index ("PB0", "PC6"...). */
export function gpioNames(spec: AvrDeviceSpec): string[] {
  const names = Array.from({ length: spec.gpioCount }, (_, i) => `GPIO${i}`);
  for (const p of spec.pins) if (p.gpio !== undefined) names[p.gpio] = p.name;
  return names;
}

/** Value of a (multi-bit) fuse field, e.g. CKSEL, from fuse bytes. */
export function fuseField(spec: AvrDeviceSpec, fuses: number[], name: string): number | null {
  for (let i = 0; i < spec.fuses.length; i++) {
    const f = spec.fuses[i].bits.find((b) => b.name === name);
    if (f) return ((fuses[i] ?? 0xff) & f.mask) >> Math.log2(f.mask & -f.mask);
  }
  return null;
}

/** Switches the target device; reloads the current program image (if any) on the new part. */
export function selectDevice(id: string): void {
  if (useSettings.getState().deviceId === id) return;
  useSettings.getState().set({ deviceId: id });
  const b = useWorkspace.getState().build;
  appendOutput('info', `Target device: ${id}${b ? ' (rebuild to re-check the program for this device)' : ''}`);
  if (b) loadProgram(b.program, b.docId, b.label, id);
  else sim({ type: 'init', deviceId: id });
}
