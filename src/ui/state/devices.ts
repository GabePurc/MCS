import { create } from 'zustand';
import { listDevices } from '../backend/api';
import type { DeviceSummary } from '../backend/types';

export const useDevices = create<{ devices: DeviceSummary[] }>(() => ({ devices: [] }));

export async function loadDevices(): Promise<void> {
  try {
    useDevices.setState({ devices: await listDevices() });
  } catch {
    // Browser preview without backend: keep the built-in default only.
    useDevices.setState({ devices: [{ id: 'attiny10', name: 'ATtiny10', family: 'tinyAVR', flashSize: 1024, sramSize: 32, package: 'SOT-23-6', coreName: 'AVRrc' }] });
  }
}
