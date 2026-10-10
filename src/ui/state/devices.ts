import { create } from 'zustand';
import { listDevices } from '../backend/api';
import type { Arch, DeviceSummary } from '../backend/types';
import { useCustomDevices } from './customDevices';

export const useDevices = create<{ devices: DeviceSummary[] }>(() => ({ devices: [] }));

/** Architecture of a device id from the device list (AVR while the list is not loaded). */
export function archOf(deviceId: string): Arch {
  return useDevices.getState().devices.find((d) => d.id === deviceId)?.arch ?? 'avr';
}

export async function loadDevices(): Promise<void> {
  try {
    // Deleted custom devices stay registered in the backend until restart: hide them.
    const custom = new Set(useCustomDevices.getState().configs.map((c) => c.id));
    const all = await listDevices();
    useDevices.setState({ devices: all.filter((d) => d.family !== 'Custom' || custom.has(d.id)) });
  } catch (e) {
    console.warn('listDevices failed', e);
    // Browser preview without backend: keep the built-in default only.
    useDevices.setState({ devices: [{ arch: 'avr', id: 'attiny10', name: 'ATtiny10', family: 'tinyAVR', flashSize: 1024, sramSize: 32, package: 'SOT-23-6', coreName: 'AVRrc' }] });
  }
}
