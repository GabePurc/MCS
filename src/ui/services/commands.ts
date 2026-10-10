/**
 * Central command registry. Menus, toolbars, context menus and keyboard shortcuts all invoke
 * commands by id, so behaviour and enablement are defined in exactly one place.
 */
import type { IconName } from '../icons';
import type { SpeedMode } from '../backend/types';
import { platform, win } from '../backend/api';
import { editorApi } from '../editor/editorApi';
import { isPanelOpen, useLayout, type PanelId } from '../state/layout';
import { useSettings } from '../state/settings';
import { useSim } from '../state/sim';
import { activeDoc, clearBreakpoints, clearOutput, docKey, toggleSourceBreakpoint, useWorkspace } from '../state/workspace';
import { openDialog } from '../state/dialogs';
import { buildActive, importHexOrElf, isBuildStale, openProgramAsMachineCode } from './build';
import { closeDocument, confirmQuit, newFile, openFileDialog, saveAll, saveDoc } from './files';
import { formatHz } from '../format';
import { openUpdateDialog } from './updater';
import { inTauri } from '../backend/api';
import { sim } from './simClient';
import { sourceToPc } from './debugInfo';
import { exportHexDialog } from './exporting';
import { archOf } from '../state/devices';

export interface CommandDef {
  id: string;
  label: string;
  icon?: IconName;
  /** Key specs like "F5", "Shift+F11", "Mod+S" (Mod = Cmd on macOS, Ctrl elsewhere). */
  keys?: string[];
  run: () => void | Promise<unknown>;
  enabled?: () => boolean;
  checked?: () => boolean;
  /** Let the focused text editor handle the key instead (edit commands). */
  editorHandles?: boolean;
}

const isMac = platform === 'macos';

export function shortcutLabel(keys: string[] | undefined): string {
  if (!keys?.length) return '';
  return keys[0].replace('Mod', isMac ? 'Cmd' : 'Ctrl');
}

const hasDoc = () => !!useWorkspace.getState().activeDocId;
const running = () => useSim.getState().running;
/** Fuses, EEPROM, ISA reference, .inc files and custom MCUs exist for AVR devices only. */
const avrSelected = () => archOf(useSettings.getState().deviceId) === 'avr';
const hasProgram = () => !!useWorkspace.getState().build;
const canBuild = () => hasDoc() && !useWorkspace.getState().building;

/** True when the active source document is not what the simulator is running. */
function needsBuild(): boolean {
  const ws = useWorkspace.getState();
  const doc = activeDoc();
  if (!doc) return !ws.build;
  return ws.build?.docId !== doc.id || isBuildStale();
}

/** Builds the active document when needed; returns false if there is nothing to run. */
async function ensureProgram(): Promise<boolean> {
  const ws = useWorkspace.getState();
  if (ws.building) return false;
  if (!needsBuild()) return !!ws.build;
  if (!activeDoc()) return !!ws.build;
  return buildActive();
}

function sourceStepping(): boolean {
  return useSettings.getState().sourceStepping && (useWorkspace.getState().build?.program.lines.length ?? 0) > 0;
}

async function step(kind: 'into' | 'over' | 'out'): Promise<void> {
  if (running()) return;
  if (needsBuild()) {
    await ensureProgram(); // a fresh load stops at the reset vector
    return;
  }
  sim({ type: 'step', kind, source: sourceStepping() });
}

function cursorLocation(): { file: string; line: number } | null {
  const doc = activeDoc();
  const api = editorApi();
  if (!doc || !api) return null;
  return { file: docKey(doc), line: api.cursorLine() };
}

/** Speed presets: fixed CPU rates from 1 Hz (watch every instruction), real-time multiples, maximum. */
export const SPEEDS: [string, number, SpeedMode][] = [
  ['1 Hz (1 cycle per second)', 1, 'clock'],
  ['10 Hz', 10, 'clock'],
  ['100 Hz', 100, 'clock'],
  ['1 kHz', 1e3, 'clock'],
  ['10 kHz', 1e4, 'clock'],
  ['100 kHz', 1e5, 'clock'],
  ['1/10 real-time', 0.1, 'realtime'],
  ['Real-time (actual MCU clock)', 1, 'realtime'],
  ['10x real-time', 10, 'realtime'],
  ['Maximum speed', 1, 'max'],
];

export function setSpeed(mode: SpeedMode, factor: number): void {
  useSettings.getState().set({ speedMode: mode, speedFactor: factor });
  sim({ type: 'setSpeed', mode, factor });
}

/** Short label for a speed setting ("1 kHz", "10x", "Real-time", "Maximum"). */
export function speedLabel(mode: SpeedMode, factor: number): string {
  if (mode === 'max') return 'Maximum';
  if (mode === 'clock') return formatHz(factor);
  if (factor === 1) return 'Real-time';
  return factor < 1 ? `1/${+(1 / factor).toPrecision(3)}x` : `${+factor.toPrecision(3)}x`;
}

export const PANELS: [PanelId, string, IconName][] = [
  ['processor', 'Processor', 'Cpu'],
  ['io', 'I/O View', 'Io'],
  ['memory', 'Memory', 'Memory'],
  ['disasm', 'Disassembly', 'Disasm'],
  ['pins', 'Pins & Stimulus', 'Pins'],
  ['wave', 'Waveform', 'Wave'],
  ['output', 'Output', 'Output'],
  ['symbols', 'Symbols', 'Symbols'],
  ['callstack', 'Call Stack', 'CallStack'],
  ['breakpoints', 'Breakpoints', 'List'],
  ['chip', 'Chip View (3D)', 'Chip3D'],
  ['info', 'Device Info', 'Info'],
  ['isa', 'Instruction Set', 'Book'],
  ['serial', 'Serial Monitor', 'Serial'],
  ['defs', 'Device Definitions (.inc)', 'Book'],
];

export const PANEL_TITLES: Record<PanelId, string> = Object.fromEntries(PANELS.map(([id, t]) => [id, t])) as Record<PanelId, string>;
export const PANEL_ICONS: Record<PanelId, IconName> = Object.fromEntries(PANELS.map(([id, , i]) => [id, i])) as Record<PanelId, IconName>;

const list: CommandDef[] = [
  // File
  { id: 'file.newAsm', label: 'New Assembly File', icon: 'NewFile', keys: ['Mod+N'], run: () => newFile('asm') },
  { id: 'file.newC', label: 'New C File', run: () => newFile('c') },
  { id: 'file.newMc', label: 'New Machine Code File', icon: 'MachineCode', run: () => newFile('mc') },
  { id: 'file.open', label: 'Open...', icon: 'Open', keys: ['Mod+O'], run: openFileDialog },
  { id: 'file.save', label: 'Save', icon: 'Save', keys: ['Mod+S'], run: () => saveDoc(), enabled: hasDoc },
  { id: 'file.saveAs', label: 'Save As...', run: () => saveDoc(activeDoc(), true), enabled: hasDoc },
  { id: 'file.saveAll', label: 'Save All', keys: ['Mod+Shift+S'], run: saveAll, enabled: hasDoc },
  { id: 'file.close', label: 'Close', keys: ['Mod+W'], run: () => closeDocument(), enabled: hasDoc },
  { id: 'file.import', label: 'Import HEX / ELF...', icon: 'Import', keys: ['Mod+I'], run: importHexOrElf },
  { id: 'file.exportHex', label: 'Export Intel HEX...', icon: 'Export', run: exportHexDialog, enabled: () => hasProgram() && avrSelected() },
  { id: 'file.exit', label: 'Exit', keys: isMac ? ['Mod+Q'] : ['Alt+F4'], run: async () => { if (await confirmQuit()) win.destroy(); } },

  // Edit (CodeMirror handles the keys itself when focused)
  { id: 'edit.undo', label: 'Undo', keys: ['Mod+Z'], editorHandles: true, run: () => editorApi()?.undo(), enabled: hasDoc },
  { id: 'edit.redo', label: 'Redo', keys: ['Mod+Y'], editorHandles: true, run: () => editorApi()?.redo(), enabled: hasDoc },
  { id: 'edit.cut', label: 'Cut', keys: ['Mod+X'], editorHandles: true, run: () => editorApi()?.cut(), enabled: hasDoc },
  { id: 'edit.copy', label: 'Copy', keys: ['Mod+C'], editorHandles: true, run: () => editorApi()?.copy(), enabled: hasDoc },
  { id: 'edit.paste', label: 'Paste', keys: ['Mod+V'], editorHandles: true, run: () => editorApi()?.paste(), enabled: hasDoc },
  { id: 'edit.selectAll', label: 'Select All', keys: ['Mod+A'], editorHandles: true, run: () => editorApi()?.selectAll(), enabled: hasDoc },
  { id: 'edit.find', label: 'Find...', icon: 'Find', keys: ['Mod+F'], editorHandles: true, run: () => editorApi()?.find(), enabled: hasDoc },
  { id: 'edit.replace', label: 'Replace...', keys: ['Mod+H'], editorHandles: true, run: () => editorApi()?.replace(), enabled: hasDoc },
  { id: 'edit.gotoLine', label: 'Go To Line...', keys: ['Mod+G'], editorHandles: true, run: () => editorApi()?.gotoLine(), enabled: hasDoc },

  // View
  ...PANELS.map(([id, label, icon]): CommandDef => ({
    id: `view.${id}`,
    label,
    icon,
    run: () => useLayout.getState().toggle(id),
    checked: () => isPanelOpen(id),
  })),
  {
    id: 'view.symbolView',
    label: 'Symbol View',
    icon: 'Symbols',
    keys: ['Mod+Shift+O'],
    run: () => useSettings.getState().set({ symbolView: !useSettings.getState().symbolView }),
    checked: () => useSettings.getState().symbolView,
  },
  { id: 'view.resetLayout', label: 'Reset Window Layout', run: () => useLayout.getState().reset() },
  { id: 'view.startPage', label: 'Start Page', run: () => useWorkspace.setState({ activeDocId: null }) },

  // Build
  { id: 'build.build', label: 'Build', icon: 'Build', keys: ['F7'], run: buildActive, enabled: canBuild },
  { id: 'build.options', label: 'Toolchain Options...', icon: 'Settings', run: () => openDialog('toolchain') },
  { id: 'build.toMachineCode', label: 'Open Program as Machine Code', icon: 'MachineCode', run: openProgramAsMachineCode, enabled: () => hasProgram() && avrSelected() },

  // Debug
  {
    id: 'debug.start',
    label: 'Start / Continue',
    icon: 'Run',
    keys: ['F5'],
    run: async () => {
      if (running()) return;
      // A run from reset is a fresh run; Continue keeps the output.
      if (useSettings.getState().clearOutputOnRun && !needsBuild() && (useSim.getState().state?.cycles ?? 0) === 0) clearOutput();
      if (await ensureProgram()) sim({ type: 'run' });
    },
    enabled: () => !running() && (hasDoc() || hasProgram()),
  },
  { id: 'debug.pause', label: 'Break All', icon: 'Pause', keys: ['F6', 'Mod+Alt+B'], run: () => sim({ type: 'pause' }), enabled: running },
  { id: 'debug.stop', label: 'Stop (Power Cycle)', icon: 'Stop', keys: ['Shift+F5'], run: () => sim({ type: 'powerCycle' }), enabled: hasProgram },
  { id: 'debug.reset', label: 'Reset MCU', icon: 'Reset', keys: ['Mod+Shift+F5'], run: () => sim({ type: 'reset' }), enabled: hasProgram },
  { id: 'debug.stepInto', label: 'Step Into', icon: 'StepInto', keys: ['F11'], run: () => step('into'), enabled: () => !running() && (hasDoc() || hasProgram()) },
  { id: 'debug.stepOver', label: 'Step Over', icon: 'StepOver', keys: ['F10'], run: () => step('over'), enabled: () => !running() && (hasDoc() || hasProgram()) },
  { id: 'debug.stepOut', label: 'Step Out', icon: 'StepOut', keys: ['Shift+F11'], run: () => step('out'), enabled: () => !running() && hasProgram() },
  {
    id: 'debug.runToCursor',
    label: 'Run To Cursor',
    icon: 'RunToCursor',
    keys: ['Mod+F10'],
    run: async () => {
      const loc = cursorLocation();
      if (!loc || running() || !(await ensureProgram())) return;
      const build = useWorkspace.getState().build;
      const { pc } = sourceToPc(build?.program, loc.file, loc.line, build?.arch);
      if (pc >= 0) sim({ type: 'runTo', pc });
    },
    enabled: () => !running() && hasDoc(),
  },
  {
    id: 'debug.toggleBreakpoint',
    label: 'Toggle Breakpoint',
    icon: 'Breakpoint',
    keys: ['F9'],
    run: () => {
      const loc = cursorLocation();
      if (loc) toggleSourceBreakpoint(loc.file, loc.line);
    },
    enabled: hasDoc,
  },
  { id: 'debug.clearBreakpoints', label: 'Delete All Breakpoints', icon: 'ClearBreakpoints', keys: ['Mod+Shift+F9'], run: clearBreakpoints },
  {
    id: 'debug.sourceStepping',
    label: 'Step by Source Line',
    run: () => useSettings.getState().set({ sourceStepping: !useSettings.getState().sourceStepping }),
    checked: () => useSettings.getState().sourceStepping,
  },
  {
    id: 'build.clearOutputOnRun',
    label: 'Clear Output on Build / Run',
    run: () => useSettings.getState().set({ clearOutputOnRun: !useSettings.getState().clearOutputOnRun }),
    checked: () => useSettings.getState().clearOutputOnRun,
  },
  ...SPEEDS.map(([label, factor, mode]): CommandDef => ({
    id: `speed.${mode}.${factor}`,
    label,
    run: () => setSpeed(mode, factor),
    checked: () => {
      const s = useSettings.getState();
      return s.speedMode === mode && (mode === 'max' || s.speedFactor === factor);
    },
  })),
  { id: 'speed.custom', label: 'Custom Speed...', icon: 'Settings', run: () => openDialog('speed') },

  // Device / tools / help
  { id: 'device.custom', label: 'Custom Microcontroller...', icon: 'Chip3D', run: () => openDialog('customDevice'), enabled: avrSelected },
  { id: 'device.fuses', label: 'Fuses & Lock Bits...', icon: 'Fuse', run: () => openDialog('fuses'), enabled: avrSelected },
  { id: 'device.supply', label: 'Supply & Clock...', icon: 'Settings', run: () => openDialog('supply') },
  { id: 'tools.toolchain', label: 'Toolchain Options...', icon: 'Settings', run: () => openDialog('toolchain') },
  { id: 'device.info', label: 'Device Info', icon: 'Info', run: () => useLayout.getState().show('info') },
  { id: 'device.chip', label: 'Chip View (3D)', icon: 'Chip3D', run: () => useLayout.getState().show('chip') },
  { id: 'help.isa', label: 'Instruction Set Reference', icon: 'Book', keys: ['F1'], run: () => useLayout.getState().show('isa'), enabled: avrSelected },
  { id: 'help.include', label: 'Device Definitions (.inc)', run: () => useLayout.getState().show('defs'), enabled: avrSelected },
  { id: 'help.toolchain', label: 'C Toolchain Setup', run: () => openDialog('toolchainHelp') },
  { id: 'help.updates', label: 'Check for Updates...', icon: 'Download', run: openUpdateDialog, enabled: () => inTauri },
  { id: 'help.about', label: 'About MCS', icon: 'App', run: () => openDialog('about') },
];

export const COMMANDS: Record<string, CommandDef> = Object.fromEntries(list.map((c) => [c.id, c]));
export const SPEED_COMMAND_IDS = [...SPEEDS.map(([, f, m]) => `speed.${m}.${f}`), 'speed.custom'];
export const PANEL_COMMAND_IDS = PANELS.map(([id]) => `view.${id}`);

/** Set in pop-out windows: commands run in the main window (it owns documents and builds). */
export const commandHooks = { remote: null as ((id: string) => void) | null };

export function runCommand(id: string): void {
  const c = COMMANDS[id];
  if (!c) return;
  if (commandHooks.remote) {
    commandHooks.remote(id);
    return;
  }
  if (c.enabled && !c.enabled()) return;
  void Promise.resolve(c.run()).catch((e) => console.error(`command ${id} failed`, e));
}

function eventSpec(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (isMac ? e.metaKey : e.ctrlKey) parts.push('Mod');
  if (e.altKey) parts.push('Alt');
  if (e.shiftKey) parts.push('Shift');
  let k = e.key;
  if (k.length === 1) k = k.toUpperCase();
  if (e.code?.startsWith('Key')) k = e.code.slice(3);
  parts.push(k);
  return parts.join('+');
}

/** Global shortcut handler. Returns true when the event was handled. */
export function handleShortcut(e: KeyboardEvent): boolean {
  const spec = eventSpec(e);
  const inEditor = (e.target as HTMLElement | null)?.closest?.('.cm-editor, input, textarea, select');
  for (const c of list) {
    if (!c.keys?.includes(spec)) continue;
    if (c.editorHandles && inEditor) return false;
    e.preventDefault();
    runCommand(c.id);
    return true;
  }
  return false;
}
