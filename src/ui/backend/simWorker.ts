/// <reference lib="webworker" />
/**
 * Browser-mode simulation host: runs the Rust session (WASM) in a worker so the UI stays
 * smooth, pacing slices like the native simulation thread does.
 */
import { instantiateCore, type WasmCore } from './wasmHost';
import type { CustomMcuConfig, SimCommand, SimOutput } from './types';

const ctx = self as unknown as DedicatedWorkerGlobalScope;
let core: WasmCore | null = null;
/** Messages that arrived before the core was ready (kept in order). */
const pending: Msg[] = [];

interface Msg {
  init?: string;
  cmd?: SimCommand;
  register?: CustomMcuConfig[];
}
let timer: ReturnType<typeof setTimeout> | null = null;

// Zero-delay yields (setTimeout(0) is clamped to >= 4 ms when nested).
const channel = new MessageChannel();
channel.port1.onmessage = () => tick();

interface SimResult {
  outputs: SimOutput[];
  idleMs: number;
}

function emit(outputs: SimOutput[]): void {
  for (const o of outputs) ctx.postMessage(o);
}

function schedule(idleMs: number): void {
  if (timer !== null) clearTimeout(timer);
  timer = null;
  if (idleMs >= 3_600_000) return;
  if (idleMs <= 0) channel.port2.postMessage(0);
  else timer = setTimeout(tick, idleMs);
}

function tick(): void {
  if (!core) return;
  const r = core.call<SimResult>({ method: 'slice' });
  emit(r.outputs);
  schedule(r.idleMs);
}

function run(cmd: SimCommand): void {
  const r = core!.call<SimResult>({ method: 'sim', cmd });
  emit(r.outputs);
  schedule(r.idleMs);
}

function handle(m: Msg): void {
  // Custom devices must exist in this WASM instance before a session uses them.
  if (m.register) core!.call({ method: 'registerCustomDevices', configs: m.register });
  if (m.cmd) run(m.cmd);
}

ctx.onmessage = async (e: MessageEvent<Msg>) => {
  if (e.data.init) {
    core = await instantiateCore(e.data.init);
    for (const m of pending.splice(0)) handle(m);
    return;
  }
  if (core) handle(e.data);
  else pending.push(e.data);
};
