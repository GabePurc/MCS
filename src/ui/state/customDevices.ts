/**
 * User-defined microcontrollers (issue #3). The configurations persist in local storage and are
 * registered with the Rust backend at start-up; the backend generates the full device spec.
 */
import { create } from 'zustand';
import { registerCustomDevices } from '../backend/api';
import type { CustomMcuConfig } from '../backend/types';
import { loadJson, saveJson } from './persist';
import { appendOutput } from './workspace';

const KEY = 'mcs.customDevices.v1';

export const useCustomDevices = create<{ configs: CustomMcuConfig[] }>(() => ({ configs: loadJson<CustomMcuConfig[]>(KEY, []) }));

/** Registers every stored configuration; reports the ones the backend rejects. */
export async function registerStoredCustomDevices(): Promise<void> {
  const configs = useCustomDevices.getState().configs;
  if (!configs.length) return;
  try {
    for (const r of await registerCustomDevices(configs)) if (r.error) appendOutput('error', `Custom device ${r.id}: ${r.error}`);
  } catch (e) {
    appendOutput('error', `Custom devices unavailable: ${e instanceof Error ? e.message : String(e)}`);
  }
}

/** Saves (adds or replaces by id) and registers a configuration; returns the backend's error, if any. */
export async function saveCustomDevice(cfg: CustomMcuConfig): Promise<string | null> {
  const others = useCustomDevices.getState().configs.filter((c) => c.id !== cfg.id);
  const configs = [...others, cfg];
  const res = await registerCustomDevices(configs);
  const err = res.find((r) => r.id === cfg.id)?.error ?? null;
  if (err) return err;
  useCustomDevices.setState({ configs });
  saveJson(KEY, configs);
  return null;
}

export function deleteCustomDevice(id: string): void {
  const configs = useCustomDevices.getState().configs.filter((c) => c.id !== id);
  useCustomDevices.setState({ configs });
  saveJson(KEY, configs);
}

/** A free id derived from a display name ("My Chip" -> "custom-my-chip"). */
export function customId(name: string, taken: (id: string) => boolean): string {
  const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'mcu';
  let id = `custom-${slug}`;
  for (let n = 2; taken(id); n++) id = `custom-${slug}-${n}`;
  return id;
}
