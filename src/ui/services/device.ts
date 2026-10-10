import { useSettings } from '../state/settings';
import { useWorkspace, appendOutput } from '../state/workspace';
import { sim } from './simClient';
import { loadProgram } from './build';
import type { AvrDeviceSpec, DeviceSpec } from '../backend/types';
import { archOf } from '../state/devices';
import { clearBreakpoints } from '../state/workspace';

/**
 * GPIO names by GPIO index ("PB0", "PC6"...). ARM indices are `port * 16 + bit` and only some of
 * them exist on a package (see `existingGpios`); the names of the others are still well-formed.
 */
export function gpioNames(spec: DeviceSpec): string[] {
  if (spec.arch === 'riscv') return Array.from({ length: spec.gpioCount }, (_, i) => `GPIO${i}`);
  if (spec.arch === 'arm') return Array.from({ length: spec.gpioCount }, (_, i) => `P${String.fromCharCode(65 + (i >> 4))}${i & 15}`);
  const names = Array.from({ length: spec.gpioCount }, (_, i) => `GPIO${i}`);
  for (const p of spec.pins) if (p.gpio !== undefined) names[p.gpio] = p.name;
  return names;
}

/** GPIO indices that exist on the package, ascending (all of them for AVR parts). */
export function existingGpios(spec: DeviceSpec): number[] {
  const set = new Set<number>();
  for (const p of spec.pins) if (p.gpio !== undefined && p.gpio !== null) set.add(p.gpio);
  return [...set].sort((a, b) => a - b);
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
  const prev = useSettings.getState().deviceId;
  if (prev === id) return;
  // Address breakpoints are in the architecture's native pc unit: they do not carry over.
  const archChanged = archOf(prev) !== archOf(id);
  if (archChanged) clearBreakpoints();
  useSettings.getState().set({ deviceId: id });
  // A program image built for another architecture cannot be reloaded on the new device.
  if (archChanged) useWorkspace.setState({ build: null });
  const b = useWorkspace.getState().build;
  appendOutput('info', `Target device: ${id}${b ? ' (rebuild to re-check the program for this device)' : ''}`);
  if (b) loadProgram(b.program, b.docId, b.label, id);
  else sim({ type: 'init', deviceId: id });
}

/** Rows of the logic analyzer: every GPIO on AVR parts, only the ones on the package on ARM (index gaps). */
export function waveformRows(spec: DeviceSpec): { pin: number; name: string }[] {
  const names = gpioNames(spec);
  const pins = spec.arch !== 'avr' ? existingGpios(spec) : names.map((_, i) => i);
  return pins.map((pin) => ({ pin, name: names[pin] }));
}
