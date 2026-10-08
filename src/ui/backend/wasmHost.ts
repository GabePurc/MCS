/**
 * Loader for the WebAssembly build of the Rust core (crates/mcs-wasm). Used when the UI runs
 * in a plain browser instead of the Tauri shell. ABI: JSON requests/responses passed through
 * linear memory (see crates/mcs-wasm/src/lib.rs).
 */
const urls = import.meta.glob('../assets/mcs.wasm', { query: '?url', import: 'default', eager: true }) as Record<string, string>;

/** URL of the bundled module, or null when it was not built (`npm run build:wasm`). */
export const wasmUrl: string | null = Object.values(urls)[0] ?? null;

export interface WasmCore {
  call<T = unknown>(req: Record<string, unknown>): T;
}

interface Exports {
  memory: WebAssembly.Memory;
  mcs_alloc(len: number): number;
  mcs_free(ptr: number, len: number): void;
  mcs_call(ptr: number, len: number): bigint;
}

export async function instantiateCore(url: string): Promise<WasmCore> {
  const bytes = await (await fetch(url)).arrayBuffer();
  const { instance } = await WebAssembly.instantiate(bytes, { env: { mcs_now_ms: () => performance.now() } });
  const ex = instance.exports as unknown as Exports;
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  return {
    call<T>(req: Record<string, unknown>): T {
      const input = enc.encode(JSON.stringify(req));
      const p = ex.mcs_alloc(input.length);
      new Uint8Array(ex.memory.buffer, p, input.length).set(input);
      const r = ex.mcs_call(p, input.length);
      ex.mcs_free(p, input.length);
      const outPtr = Number(r >> 32n);
      const outLen = Number(r & 0xffffffffn);
      const text = dec.decode(new Uint8Array(ex.memory.buffer, outPtr, outLen));
      ex.mcs_free(outPtr, outLen);
      const res = JSON.parse(text) as { ok: boolean; result?: T; error?: string };
      if (!res.ok) throw new Error(res.error);
      return res.result as T;
    },
  };
}

let mainCore: Promise<WasmCore> | null = null;

/** Main-thread instance for build/disassembly services. */
export function core(): Promise<WasmCore> {
  if (!wasmUrl) return Promise.reject(new Error('The WebAssembly core is not built. Run `npm run build:wasm`, or start the desktop app with `npm run dev`.'));
  mainCore ??= instantiateCore(wasmUrl);
  return mainCore;
}
