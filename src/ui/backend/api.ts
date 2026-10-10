/**
 * Thin, typed facade over the Tauri backend (Rust commands, native dialogs, window controls).
 * Everything platform-specific in the UI goes through this module.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { open, save, ask, message } from '@tauri-apps/plugin-dialog';
import type { BuildOutcome, CustomMcuConfig, CustomPreview, CustomRegistration, DeviceSummary, DisasmLine, InsnInfo, McAnnotations, SimCommand, SimOutput, ToolchainInfo } from './types';
import { core, wasmUrl } from './wasmHost';

/** True when running inside the Tauri shell (false in a plain browser during `npm run dev:web`). */
export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export const platform: 'windows' | 'macos' | 'linux' = /Win/.test(navigator.userAgent) ? 'windows' : /Mac/.test(navigator.userAgent) ? 'macos' : 'linux';

function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) return Promise.reject(new Error('This feature needs the desktop app (start it with `npm run dev`).'));
  return invoke<T>(cmd, args);
}

/** Browser mode: same Rust core compiled to WebAssembly. */
async function wasm<T>(req: Record<string, unknown>): Promise<T> {
  return (await core()).call<T>(req);
}

/** True when the simulator backend is usable (desktop app, or browser with the WASM core). */
export const backendAvailable = inTauri || wasmUrl !== null;

// ---------------------------------------------------------------- simulator
let browserWorker: Worker | null = null;

export function simAttach(onOutput: (o: SimOutput) => void): Promise<void> {
  if (!inTauri) {
    if (!wasmUrl) return Promise.reject(new Error('No backend: run the desktop app (`npm run dev`) or build the WebAssembly core (`npm run build:wasm`).'));
    browserWorker = new Worker(new URL('./simWorker.ts', import.meta.url), { type: 'module' });
    browserWorker.onmessage = (e: MessageEvent<SimOutput>) => onOutput(e.data);
    browserWorker.postMessage({ init: new URL(wasmUrl, location.href).href });
    if (customConfigs.length) browserWorker.postMessage({ register: customConfigs });
    return Promise.resolve();
  }
  const channel = new Channel<SimOutput>();
  channel.onmessage = onOutput;
  return call('sim_attach', { channel });
}

export function simCommand(cmd: SimCommand): void {
  if (browserWorker) {
    browserWorker.postMessage({ cmd });
    return;
  }
  call('sim_command', { cmd }).catch((e) => console.error('sim_command failed', e));
}

// ---------------------------------------------------------------- custom devices
/** Last registered set (browser mode re-sends it to a simulation worker started later). */
let customConfigs: CustomMcuConfig[] = [];

/** Registers user-defined devices with the backend (and the browser simulation worker). */
export async function registerCustomDevices(configs: CustomMcuConfig[]): Promise<CustomRegistration[]> {
  if (inTauri) return call<CustomRegistration[]>('register_custom_devices', { configs });
  customConfigs = configs;
  browserWorker?.postMessage({ register: configs });
  return wasm<CustomRegistration[]>({ method: 'registerCustomDevices', configs });
}

export async function customDevicePreview(config: CustomMcuConfig): Promise<{ ok?: CustomPreview; error?: string }> {
  if (inTauri) return call<CustomPreview>('custom_device_preview', { config }).then((ok) => ({ ok }), (e: unknown) => ({ error: String(e) }));
  return wasm<{ ok?: CustomPreview; error?: string }>({ method: 'customDevicePreview', config });
}

export const customDeviceDefaults = () => (inTauri ? call<CustomMcuConfig>('custom_device_defaults') : wasm<CustomMcuConfig>({ method: 'customDeviceDefaults' }));

// ---------------------------------------------------------------- build
export const listDevices = () => (inTauri ? call<DeviceSummary[]>('list_devices') : wasm<DeviceSummary[]>({ method: 'listDevices' }));

export const buildAsm = (source: string, fileName: string, filePath: string | null, deviceId: string) =>
  inTauri ? call<BuildOutcome>('build_asm', { source, fileName, filePath, deviceId }) : wasm<BuildOutcome>({ method: 'buildAsm', source, fileName, deviceId });

export const buildMachineCode = (source: string, fileName: string, filePath: string | null, deviceId: string) =>
  inTauri
    ? call<BuildOutcome>('build_machine_code', { source, fileName, filePath, deviceId })
    : wasm<BuildOutcome>({ method: 'buildMachineCode', source, fileName, deviceId });

export const machineCodeHints = (source: string, deviceId: string) =>
  inTauri ? call<McAnnotations>('machine_code_hints', { source, deviceId }) : wasm<McAnnotations>({ method: 'machineCodeHints', source, deviceId });

export const programToMachineCode = (deviceId: string, flash: number[] | Uint8Array, used: number, labels: Record<number, string>, title: string) =>
  inTauri
    ? call<string>('program_to_machine_code', { deviceId, flash: Array.from(flash), used, labels, title })
    : wasm<string>({ method: 'programToMachineCode', deviceId, flash: Array.from(flash), used, labels, title });

export const buildC = (args: { source: string; fileName: string; filePath: string | null; deviceId: string; optimize: string; extraFlags: string[]; gccPath: string | null }) =>
  call<BuildOutcome>('build_c', args);

export const importProgram = (path: string, deviceId: string) => {
  if (inTauri) return call<BuildOutcome>('import_program', { path, deviceId });
  const f = webFiles.get(path);
  if (!f) return Promise.reject(new Error(`${path} is not available`));
  return wasm<BuildOutcome>({ method: 'importProgram', bytes: Array.from(f.bytes), fileName: f.name, deviceId });
};

/** Parses an ELF / Intel HEX image held in memory (bundled examples). */
export const importProgramBytes = (bytes: Uint8Array, fileName: string, deviceId: string) =>
  inTauri
    ? call<BuildOutcome>('import_program_bytes', { bytes: Array.from(bytes), fileName, deviceId })
    : wasm<BuildOutcome>({ method: 'importProgram', bytes: Array.from(bytes), fileName, deviceId });

export const detectToolchain = (gccPath: string | null) => call<ToolchainInfo | null>('detect_toolchain', { gccPath });

/** Disassembles the programmed part of `flash` (trailing erased words are left out: large parts have megabytes of them). */
export function disassemble(deviceId: string, flash: number[] | Uint8Array, labels: Record<number, string>): Promise<DisasmLine[]> {
  let end = flash.length;
  while (end > 512 && flash[end - 1] === 0xff) end--;
  const bytes = Array.from(flash.slice(0, Math.min(flash.length, (end + 3) & ~1)));
  return inTauri ? call<DisasmLine[]>('disassemble', { deviceId, flash: bytes, labels }) : wasm<DisasmLine[]>({ method: 'disassemble', deviceId, flash: bytes, labels });
}

export const instructionSet = (deviceId: string) => (inTauri ? call<InsnInfo[]>('instruction_set', { deviceId }) : wasm<InsnInfo[]>({ method: 'instructionSet', deviceId }));

export const defInclude = (deviceId: string) => (inTauri ? call<[string, string] | null>('def_include', { deviceId }) : wasm<[string, string] | null>({ method: 'defInclude', deviceId }));

// ---------------------------------------------------------------- files
/** Browser mode keeps opened files in memory under "web:<name>" pseudo paths. */
const webFiles = new Map<string, { name: string; bytes: Uint8Array }>();

function download(name: string, data: string): void {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([data], { type: 'text/plain' }));
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

export const readTextFile = (path: string) => {
  if (inTauri) return call<string>('read_text_file', { path });
  const f = webFiles.get(path);
  return f ? Promise.resolve(new TextDecoder().decode(f.bytes)) : Promise.reject(new Error('File not available in the browser'));
};

export const writeTextFile = (path: string, content: string) => {
  if (inTauri) return call<void>('write_text_file', { path, content });
  const name = path.replace(/^web:/, '');
  webFiles.set(path, { name, bytes: new TextEncoder().encode(content) });
  download(name, content);
  return Promise.resolve();
};

export const exportHex = async (path: string, flash: number[] | Uint8Array, used: number) => {
  if (inTauri) return call<void>('export_hex', { path, flash: Array.from(flash), used });
  download(path.replace(/^web:/, ''), await wasm<string>({ method: 'toIntelHex', flash: Array.from(flash), used }));
};

export interface FileFilter {
  name: string;
  extensions: string[];
}

export async function pickFile(title: string, filters: FileFilter[]): Promise<string | null> {
  if (!inTauri) return pickBrowserFile(filters);
  const r = await open({ title, filters, multiple: false, directory: false });
  return typeof r === 'string' ? r : null;
}

export async function pickSavePath(title: string, defaultPath: string | undefined, filters: FileFilter[]): Promise<string | null> {
  if (!inTauri) {
    const name = window.prompt(title, (defaultPath ?? 'untitled').replace(/^web:/, '').split(/[\\/]/).pop());
    return name ? `web:${name}` : null;
  }
  return (await save({ title, defaultPath, filters })) ?? null;
}

function pickBrowserFile(filters: FileFilter[]): Promise<string | null> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = filters.flatMap((f) => f.extensions.filter((e) => e !== '*').map((e) => `.${e}`)).join(',');
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return resolve(null);
      const key = `web:${file.name}`;
      webFiles.set(key, { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()) });
      resolve(key);
    };
    input.click();
  });
}

export async function confirmDialog(text: string, title = 'MCS'): Promise<boolean> {
  if (!inTauri) return window.confirm(text);
  return ask(text, { title, kind: 'warning' });
}

export async function infoDialog(text: string, title = 'MCS'): Promise<void> {
  if (!inTauri) return window.alert(text);
  await message(text, { title, kind: 'info' });
}

/** Opens a web page in the system browser. */
export async function openExternal(url: string): Promise<void> {
  if (!/^https?:\/\//.test(url)) return;
  if (!inTauri) {
    window.open(url, '_blank', 'noopener');
    return;
  }
  const { openUrl } = await import('@tauri-apps/plugin-opener');
  await openUrl(url);
}

// ---------------------------------------------------------------- window
export const win = {
  minimize: () => inTauri && void getCurrentWindow().minimize(),
  toggleMaximize: () => inTauri && void getCurrentWindow().toggleMaximize(),
  close: () => inTauri && void getCurrentWindow().close(),
  destroy: () => inTauri && void getCurrentWindow().destroy(),
  startDragging: () => inTauri && void getCurrentWindow().startDragging(),
  isMaximized: async () => (inTauri ? getCurrentWindow().isMaximized() : false),
  onResized: (cb: () => void) => (inTauri ? getCurrentWindow().onResized(cb) : Promise.resolve(() => {})),
  onFocusChanged: (cb: (focused: boolean) => void) => (inTauri ? getCurrentWindow().onFocusChanged((e) => cb(e.payload)) : Promise.resolve(() => {})),
  /** Registers a close guard; `allow` resolves whether the window may close. */
  onCloseRequested: (allow: () => Promise<boolean>) =>
    inTauri
      ? getCurrentWindow().onCloseRequested(async (e) => {
          if (!(await allow())) e.preventDefault();
        })
      : Promise.resolve(() => {}),
};
