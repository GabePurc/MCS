import { create } from 'zustand';
import { listDevices } from '../backend/api';
import type { DeviceSummary } from '../backend/types';
import { useCustomDevices } from './customDevices';

export const useDevices = create<{ devices: DeviceSummary[] }>(() => ({ devices: [] }));

export async function loadDevices(): Promise<void> {
  try {
    // Deleted custom devices stay registered in the backend until restart: hide them.
    const custom = new Set(useCustomDevices.getState().configs.map((c) => c.id));
    const all = await listDevices();
    useDevices.setState({ devices: all.filter((d) => d.family !== 'Custom' || custom.has(d.id)) });
  } catch (e) {
    console.warn('listDevices failed', e);
    // Browser preview without backend: keep the built-in default only.
    useDevices.setState({ devices: [{ id: 'attiny10', name: 'ATtiny10', family: 'tinyAVR', flashSize: 1024, sramSize: 32, package: 'SOT-23-6', coreName: 'AVRrc' }] });
  }
}
