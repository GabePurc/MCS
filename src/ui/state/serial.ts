/**
 * Serial Monitor state: received bytes (capped), display options and the line settings per
 * device (persisted). Bytes arrive with every machine state (see simClient.serialTaps).
 */
import { create } from 'zustand';
import type { DeviceSpec, SerialConfig } from '../backend/types';
import { loadJson, saveJson } from './persist';
import { serialTaps, sim } from '../services/simClient';
import { useSim } from './sim';

const MAX_BYTES = 256 * 1024;
const KEY = 'mcs.serial.v1';

interface Prefs {
  /** Line settings by device id. */
  configs: Record<string, SerialConfig>;
  hex: boolean;
  lineEnding: '' | '\n' | '\r' | '\r\n';
  autoscroll: boolean;
}

interface SerialStore extends Prefs {
  bytes: Uint8Array;
  length: number;
  /** Bumped on every change (cheap subscription for the view). */
  version: number;
}

const prefs = loadJson<Prefs>(KEY, { configs: {}, hex: false, lineEnding: '\n', autoscroll: true });

export const useSerial = create<SerialStore>(() => ({ ...prefs, bytes: new Uint8Array(4096), length: 0, version: 0 }));

function savePrefs(): void {
  const { configs, hex, lineEnding, autoscroll } = useSerial.getState();
  saveJson(KEY, { configs, hex, lineEnding, autoscroll });
}

export function setSerialPrefs(patch: Partial<Pick<Prefs, 'hex' | 'lineEnding' | 'autoscroll'>>): void {
  useSerial.setState(patch);
  savePrefs();
}

function append(data: number[]): void {
  const s = useSerial.getState();
  let { bytes, length } = s;
  if (length + data.length > bytes.length) {
    if (length + data.length > MAX_BYTES) {
      // Keep the newest half.
      const keep = Math.max(0, MAX_BYTES / 2 - data.length);
      bytes = bytes.slice(length - keep, length);
      length = keep;
    }
    const grown = new Uint8Array(Math.min(MAX_BYTES, Math.max(bytes.length * 2, length + data.length)));
    grown.set(bytes.subarray(0, length));
    bytes = grown;
  }
  bytes.set(data, length);
  useSerial.setState({ bytes, length: length + data.length, version: s.version + 1 });
}

export function clearSerial(): void {
  useSerial.setState((s) => ({ length: 0, version: s.version + 1 }));
}

/** Default line settings: the device's USART pins when it has one. */
export function defaultSerialConfig(spec: DeviceSpec): SerialConfig {
  const pin = (fn: string) => spec.pins.find((p) => p.functions.includes(fn))?.gpio ?? null;
  if (spec.arch === 'arm') {
    // USART2 is the ST-LINK virtual COM port on Nucleo boards; fall back to the first USART/UART.
    const uart = ['USART2', 'USART1', 'USART3', 'UART4', 'UART5', 'LPUART1'].find((u) => pin(`${u}_TX`) !== null && pin(`${u}_RX`) !== null);
    return uart ? { monitor: pin(`${uart}_TX`), inject: pin(`${uart}_RX`), baud: 115200, dataBits: 8, parity: 0, stopBits: 1 } : { monitor: null, inject: null, baud: 115200, dataBits: 8, parity: 0, stopBits: 1 };
  }
  return { monitor: pin('TXD'), inject: pin('RXD'), baud: 9600, dataBits: 8, parity: 0, stopBits: 1 };
}

export function serialConfigFor(spec: DeviceSpec): SerialConfig {
  return useSerial.getState().configs[spec.id] ?? defaultSerialConfig(spec);
}

export function setSerialConfig(spec: DeviceSpec, config: SerialConfig): void {
  useSerial.setState((s) => ({ configs: { ...s.configs, [spec.id]: config } }));
  savePrefs();
  sim({ type: 'setSerial', config });
}

export function sendSerial(text: string): void {
  const s = useSerial.getState();
  const bytes = Array.from(new TextEncoder().encode(text + s.lineEnding));
  if (bytes.length) sim({ type: 'serialSend', bytes });
}

let started = false;

/** Collects received bytes and re-applies the line settings whenever the machine is recreated. */
export function startSerial(): void {
  if (started) return;
  started = true;
  serialTaps.add(append);
  let lastSpec: DeviceSpec | null = null;
  const apply = (spec: DeviceSpec | null) => {
    if (!spec || spec === lastSpec) return;
    lastSpec = spec;
    sim({ type: 'setSerial', config: serialConfigFor(spec) });
  };
  apply(useSim.getState().spec);
  useSim.subscribe((s) => apply(s.spec));
}
