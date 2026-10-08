import { create } from 'zustand';
import type { AvrDeviceSpec, MachineState, StopInfo } from '../backend/types';


export interface SimStore {
  spec: AvrDeviceSpec | null;
  state: MachineState | null;
  /** State captured at the previous stop; values differing from it are shown in red. */
  baseline: MachineState | null;
  /** Last state received while stopped (becomes the next baseline). */
  lastStopped: MachineState | null;
  running: boolean;
  lastStop: StopInfo | null;
  /** Program memory as last reported by the worker. */
  flash: Uint8Array | null;
  /** EEPROM as last reported. */
  eeprom: Uint8Array | null;
  stopwatch: { cycles: number; time: number };
  /** Incremented whenever the user should be shown the current PC (stop events). */
  revealSeq: number;
  error: string | null;
}

export const useSim = create<SimStore>(() => ({
  spec: null,
  state: null,
  baseline: null,
  lastStopped: null,
  running: false,
  lastStop: null,
  flash: null,
  eeprom: null,
  stopwatch: { cycles: 0, time: 0 },
  revealSeq: 0,
  error: null,
}));

export function resetStopwatch(): void {
  const s = useSim.getState().state;
  useSim.setState({ stopwatch: { cycles: s?.cycles ?? 0, time: s?.timeSec ?? 0 } });
}
