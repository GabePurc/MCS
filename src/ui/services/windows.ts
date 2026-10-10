/**
 * Pop-out windows: any tool panel can open in its own OS window (desktop: a Tauri webview
 * window; browser: a popup). The main window stays the single owner of the simulator, the
 * documents and the build; pop-outs mirror its state through a small message bridge (Tauri
 * events on the desktop, BroadcastChannel in the browser) and send their actions back.
 *
 * main -> pop-out: raw simulator outputs, workspace/settings changes, snapshots, close requests.
 * pop-out -> main: hello (snapshot request), simulator commands, workspace edits (breakpoints,
 * navigation), settings edits, UI commands (F5, F10...), show-panel requests, bye.
 */
import { inTauri } from '../backend/api';
import type { DeviceSpec, Diagnostic, LoadedProgram, RawMachineState, SimCommand, SimOutput } from '../backend/types';
import { useLayout, windowHooks, type PanelId } from '../state/layout';
import { useSettings, type Settings } from '../state/settings';
import { useSim } from '../state/sim';
import { trace } from '../state/trace';
import { requestGoto, useWorkspace, type Breakpoint, type Doc, type OutputLine } from '../state/workspace';
import { SymbolIndex, sameFile } from './debugInfo';
import { commandHooks, PANEL_TITLES, runCommand } from './commands';
import { handleOutput, latest, outputTaps, setMirror, sim } from './simClient';
import { openPath } from './files';

const params = new URLSearchParams(typeof location === 'undefined' ? '' : location.search);
/** Panel shown by this window when it is a pop-out. */
export const popoutPanel = (params.get('popout') as PanelId | null) ?? null;
const sid = params.get('sid') ?? Math.random().toString(36).slice(2, 10);
const selfId = popoutPanel ? `popout-${popoutPanel}` : 'main';

// ------------------------------------------------------------------------------- messages

interface WsBuild {
  program: LoadedProgram;
  docId: string | null;
  label: string;
  time: number;
}

/** Mirrored workspace fields. */
interface WsPatch {
  build?: WsBuild | null;
  breakpoints?: Breakpoint[];
  docs?: Doc[];
  activeDocId?: string | null;
  diagnostics?: Diagnostic[];
  disasmGoto?: { pc: number; seq: number } | null;
  building?: boolean;
  output?: OutputLine[];
  outputAppend?: OutputLine[];
}

type SettingsPatch = Partial<Settings>;

type Msg =
  | { k: 'hello'; panel: PanelId }
  | { k: 'snapshot'; spec: DeviceSpec | null; state: RawMachineState | null; trace: ReturnType<typeof trace.export>; ws: WsPatch; settings: SettingsPatch }
  | { k: 'out'; o: SimOutput }
  | { k: 'ws'; patch: WsPatch }
  | { k: 'settings'; patch: SettingsPatch }
  | { k: 'close' }
  | { k: 'sim'; cmd: SimCommand }
  | { k: 'ws-up'; patch: WsPatch }
  | { k: 'settings-up'; patch: SettingsPatch }
  | { k: 'goto'; file: string; line: number; docId: string | null }
  | { k: 'show'; panel: PanelId }
  | { k: 'command'; id: string }
  | { k: 'bye'; panel: PanelId; dock: boolean };

interface Envelope {
  sid: string;
  from: string;
  to: string;
  m: Msg;
}

type Send = (to: string, m: Msg) => void;
let send: Send = () => {};

async function openTransport(onMsg: (m: Msg, from: string) => void): Promise<void> {
  const deliver = (env: Envelope) => {
    if (env && env.sid === sid && env.to === selfId && env.from !== selfId) onMsg(env.m, env.from);
  };
  if (inTauri) {
    const { emitTo, listen } = await import('@tauri-apps/api/event');
    await listen<Envelope>('mcs-bridge', (e) => deliver(e.payload));
    send = (to, m) => void emitTo(to, 'mcs-bridge', { sid, from: selfId, to, m } satisfies Envelope).catch(() => {});
  } else {
    const bc = new BroadcastChannel(`mcs-bridge-${sid}`);
    bc.onmessage = (e: MessageEvent<Envelope>) => deliver(e.data);
    send = (to, m) => bc.postMessage({ sid, from: selfId, to, m } satisfies Envelope);
  }
}

// ------------------------------------------------------------------------------- shared helpers

const SETTINGS_KEYS: (keyof Settings)[] = ['deviceId', 'vcc', 'speedMode', 'speedFactor', 'sourceStepping', 'editorFontSize'];

function settingsPatch(s: Settings, prev?: Settings): SettingsPatch {
  const out: Record<string, unknown> = {};
  for (const k of SETTINGS_KEYS) if (!prev || s[k] !== prev[k]) out[k] = s[k];
  return out as SettingsPatch;
}

function wsBuild(b: ReturnType<typeof useWorkspace.getState>['build']): WsBuild | null {
  return b ? { program: b.program, docId: b.docId, label: b.label, time: b.time } : null;
}

/** Workspace fields that changed between two states (all of them without `prev`). */
function wsPatch(s: ReturnType<typeof useWorkspace.getState>, prev?: ReturnType<typeof useWorkspace.getState>): WsPatch {
  const p: WsPatch = {};
  if (!prev || s.build !== prev.build) p.build = wsBuild(s.build);
  if (!prev || s.breakpoints !== prev.breakpoints) p.breakpoints = s.breakpoints;
  if (!prev || s.docs !== prev.docs) p.docs = s.docs;
  if (!prev || s.activeDocId !== prev.activeDocId) p.activeDocId = s.activeDocId;
  if (!prev || s.diagnostics !== prev.diagnostics) p.diagnostics = s.diagnostics;
  if (!prev || s.disasmGoto !== prev.disasmGoto) p.disasmGoto = s.disasmGoto;
  if (!prev || s.building !== prev.building) p.building = s.building;
  if (!prev || s.output !== prev.output) {
    const last = prev?.output[prev.output.length - 1];
    const at = last ? s.output.findIndex((l) => l.id === last.id) : -1;
    if (prev && last && at >= 0) p.outputAppend = s.output.slice(at + 1);
    else p.output = s.output;
  }
  return p;
}

let applyingRemote = false;

function applyWs(p: WsPatch): void {
  applyingRemote = true;
  try {
    const patch: Record<string, unknown> = { ...p };
    delete patch.outputAppend;
    if (p.build !== undefined) patch.build = p.build ? { ...p.build, symbols: new SymbolIndex(p.build.program) } : null;
    if (p.outputAppend) patch.output = [...useWorkspace.getState().output, ...p.outputAppend].slice(-2000);
    useWorkspace.setState(patch);
  } finally {
    applyingRemote = false;
  }
}

function applySettings(p: SettingsPatch): void {
  applyingRemote = true;
  try {
    useSettings.setState(p);
  } finally {
    applyingRemote = false;
  }
}

// ------------------------------------------------------------------------------- main window

const browserPopups = new Map<PanelId, Window>();

function popoutUrl(panel: PanelId): string {
  return `index.html?popout=${panel}&sid=${sid}`;
}

function popoutSize(panel: PanelId): { width: number; height: number } {
  const r = useLayout.getState().floatRects[panel];
  return { width: Math.round(r?.w ?? 560), height: Math.round((r?.h ?? 460) + 8) };
}

async function openTauriWindow(panel: PanelId): Promise<void> {
  const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
  const label = `popout-${panel}`;
  const existing = await WebviewWindow.getByLabel(label);
  if (existing) {
    await existing.setFocus();
    return;
  }
  const { width, height } = popoutSize(panel);
  const w = new WebviewWindow(label, {
    url: popoutUrl(panel),
    title: `${PANEL_TITLES[panel]} - MCS`,
    width,
    height,
    minWidth: 280,
    minHeight: 180,
    decorations: false,
    shadow: true,
    backgroundColor: '#cbd8e8',
  });
  void w.once('tauri://error', () => useLayout.getState().popoutClosed(panel, true));
}

/** Main window: installs the pop-out window manager and the state forwarders. */
export async function initMainWindowBridge(): Promise<void> {
  const popped = () => useLayout.getState().popped;
  const toAll = (m: Msg) => {
    for (const p of popped()) send(`popout-${p}`, m);
  };

  windowHooks.popOut = (panel) => {
    if (inTauri) {
      void openTauriWindow(panel).catch(() => useLayout.getState().popoutClosed(panel, true));
      return true;
    }
    const { width, height } = popoutSize(panel);
    const left = Math.round(window.screenX + 80);
    const top = Math.round(window.screenY + 80);
    const w = window.open(popoutUrl(panel), `mcs-${sid}-${panel}`, `popup,width=${width},height=${height},left=${left},top=${top}`);
    if (!w) return false;
    browserPopups.set(panel, w);
    return true;
  };
  windowHooks.focusPopout = (panel) => {
    if (inTauri) void openTauriWindow(panel);
    else browserPopups.get(panel)?.focus();
  };
  windowHooks.closePopout = (panel) => {
    send(`popout-${panel}`, { k: 'close' });
    browserPopups.delete(panel);
  };

  await openTransport((m, from) => {
    switch (m.k) {
      case 'hello': {
        const st = latest.state && useSim.getState().flash ? { ...latest.state, flash: Array.from(useSim.getState().flash!), traceCycles: [], traceLevels: [], messages: [], stop: undefined } : latest.state;
        send(from, { k: 'snapshot', spec: latest.spec, state: st, trace: trace.export(100_000), ws: wsPatch(useWorkspace.getState()), settings: settingsPatch(useSettings.getState()) });
        return;
      }
      case 'sim':
        sim(m.cmd);
        return;
      case 'ws-up':
        useWorkspace.setState(m.patch as Record<string, unknown>);
        return;
      case 'settings-up':
        useSettings.getState().set(m.patch);
        return;
      case 'goto': {
        const ws = useWorkspace.getState();
        const doc = ws.docs.find((d) => d.id === m.docId) ?? ws.docs.find((d) => sameFile(m.file, d.path ?? d.name));
        if (doc) requestGoto(doc.id, m.line);
        else if (/[\\/]/.test(m.file)) void openPath(m.file).then((ok) => ok && requestGoto(useWorkspace.getState().activeDocId!, m.line));
        void focusMainWindow();
        return;
      }
      case 'show':
        useLayout.getState().show(m.panel);
        return;
      case 'command':
        runCommand(m.id);
        return;
      case 'bye':
        browserPopups.delete(m.panel);
        useLayout.getState().popoutClosed(m.panel, m.dock);
        return;
    }
  });

  outputTaps.add((o) => {
    if (popped().length) toAll({ k: 'out', o });
  });
  useWorkspace.subscribe((s, p) => {
    if (!popped().length) return;
    const patch = wsPatch(s, p);
    if (Object.keys(patch).length) toAll({ k: 'ws', patch });
  });
  useSettings.subscribe((s, p) => {
    if (!popped().length) return;
    const patch = settingsPatch(s, p);
    if (Object.keys(patch).length) toAll({ k: 'settings', patch });
  });
  window.addEventListener('pagehide', () => {
    for (const p of popped()) send(`popout-${p}`, { k: 'close' });
  });
}

async function focusMainWindow(): Promise<void> {
  if (!inTauri) {
    window.focus();
    return;
  }
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().setFocus();
}

// ------------------------------------------------------------------------------- pop-out window

let saidBye = false;

/** Tells the main window this pop-out is going away (`dock` re-docks the panel there). */
export function leavePopout(dock: boolean): void {
  if (!popoutPanel || saidBye) return;
  saidBye = true;
  send('main', { k: 'bye', panel: popoutPanel, dock });
}

/** Pop-out window: mirrors the main window's state; resolves once the first snapshot arrived. */
export async function initPopoutBridge(panel: PanelId, close: () => void): Promise<void> {
  setMirror((cmd) => send('main', { k: 'sim', cmd }));
  commandHooks.remote = (id) => send('main', { k: 'command', id });
  windowHooks.remoteShow = (p) => send('main', { k: 'show', panel: p });

  let resolve: () => void = () => {};
  const ready = new Promise<void>((r) => (resolve = r));
  await openTransport((m) => {
    switch (m.k) {
      case 'snapshot':
        if (m.spec) handleOutput({ type: 'device', spec: m.spec });
        trace.clear();
        if (m.trace.cycles.length) trace.append(Float64Array.from(m.trace.cycles), Uint32Array.from(m.trace.levels), m.trace.endCycle, m.trace.hz, m.trace.words);
        if (m.state) handleOutput({ type: 'state', state: m.state });
        applyWs(m.ws);
        applySettings(m.settings);
        resolve();
        return;
      case 'out':
        handleOutput(m.o);
        return;
      case 'ws':
        applyWs(m.patch);
        return;
      case 'settings':
        applySettings(m.patch);
        return;
      case 'close':
        saidBye = true;
        close();
        return;
    }
  });

  // Local edits flow back to the main window.
  useWorkspace.subscribe((s, p) => {
    if (applyingRemote) return;
    if (s.goto && s.goto !== p.goto) {
      const doc = s.docs.find((d) => d.id === s.goto!.docId);
      send('main', { k: 'goto', docId: s.goto.docId, file: doc?.path ?? doc?.name ?? '', line: s.goto.line });
    }
    const up: WsPatch = {};
    if (s.breakpoints !== p.breakpoints) up.breakpoints = s.breakpoints;
    if (s.disasmGoto !== p.disasmGoto) up.disasmGoto = s.disasmGoto;
    if (s.output !== p.output && s.output.length === 0) up.output = [];
    if (Object.keys(up).length) send('main', { k: 'ws-up', patch: up });
  });
  useSettings.subscribe((s, p) => {
    if (applyingRemote) return;
    const patch = settingsPatch(s, p);
    if (Object.keys(patch).length) send('main', { k: 'settings-up', patch });
  });
  window.addEventListener('pagehide', () => leavePopout(false));

  // The main window may still be starting: repeat the greeting until it answers.
  let tries = 0;
  const hello = () => {
    if (useSim.getState().spec || tries++ > 40) return;
    send('main', { k: 'hello', panel });
    setTimeout(hello, 250);
  };
  hello();
  return ready;
}
