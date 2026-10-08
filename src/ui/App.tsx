import { useEffect, useState, type JSX } from 'react';
import { IconDefs } from './icons';
import { TitleBar } from './chrome/TitleBar';
import { MenuBar } from './chrome/MenuBar';
import { Toolbar } from './chrome/Toolbar';
import { StatusBar } from './chrome/StatusBar';
import { DockLayout } from './dock/DockLayout';
import { EditorArea } from './editor/EditorArea';
import { ContextMenuHost } from './controls/Menu';
import { TooltipHost } from './controls/Tooltip';
import { DialogHost } from './dialogs/Dialogs';
import { ProcessorPanel } from './panels/ProcessorPanel';
import { IoViewPanel } from './panels/IoViewPanel';
import { MemoryPanel } from './panels/MemoryPanel';
import { DisassemblyPanel } from './panels/DisassemblyPanel';
import { PinsPanel } from './panels/PinsPanel';
import { WaveformPanel } from './panels/WaveformPanel';
import { OutputPanel } from './panels/OutputPanel';
import { SymbolsPanel } from './panels/SymbolsPanel';
import { CallStackPanel } from './panels/CallStackPanel';
import { BreakpointsPanel } from './panels/BreakpointsPanel';
import type { PanelId } from './state/layout';
import { handleShortcut } from './services/commands';
import { connectSim, sim } from './services/simClient';
import { confirmQuit, openPath, restoreSession } from './services/files';
import { pcToSource, sameFile } from './services/debugInfo';
import { useSettings } from './state/settings';
import { useSim } from './state/sim';
import { appendOutput, enabledBreakpointPcs, requestGoto, useWorkspace } from './state/workspace';
import { loadDevices } from './state/devices';
import { backendAvailable, inTauri, win } from './backend/api';

let started = false;

const PANELS: Record<PanelId, () => JSX.Element> = {
  processor: ProcessorPanel,
  io: IoViewPanel,
  memory: MemoryPanel,
  disasm: DisassemblyPanel,
  pins: PinsPanel,
  wave: WaveformPanel,
  output: OutputPanel,
  symbols: SymbolsPanel,
  callstack: CallStackPanel,
  breakpoints: BreakpointsPanel,
};

const renderPanel = (id: PanelId) => {
  const P = PANELS[id];
  return <P />;
};

export function App(): JSX.Element {
  const [maximized, setMaximized] = useState(false);
  const [focused, setFocused] = useState(true);

  // Start-up: backend connection, device, session restore.
  useEffect(() => {
    if (started) return; // React StrictMode mounts twice in development
    started = true;
    const s = useSettings.getState();
    void loadDevices();
    void connectSim();
    sim({ type: 'init', deviceId: s.deviceId });
    sim({ type: 'setSpeed', mode: s.speedMode, factor: s.speedFactor });
    if (s.vcc !== 5) sim({ type: 'setVcc', volts: s.vcc });
    void restoreSession();
    if (!backendAvailable) appendOutput('warning', 'Running in a plain browser without the WebAssembly core: start the desktop app with `npm run dev` (or `npm run web`).');
    else if (!inTauri) appendOutput('info', 'MCS (web build) ready - the Rust simulator runs as WebAssembly. C compilation and file paths need the desktop app.');
    else appendOutput('info', 'MCS ready. Open an example from the Start Page or File > Open Example.');
  }, []);

  // Keyboard shortcuts.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => handleShortcut(e);
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  // Window state and close guard.
  useEffect(() => {
    const unsubs: Promise<() => void>[] = [
      win.onResized(() => void win.isMaximized().then(setMaximized)),
      win.onFocusChanged(setFocused),
      win.onCloseRequested(confirmQuit),
    ];
    void win.isMaximized().then(setMaximized);
    return () => unsubs.forEach((u) => void u.then((f) => f()));
  }, []);

  // Keep the simulator's breakpoints in sync with the workspace.
  useEffect(
    () =>
      useWorkspace.subscribe((s, p) => {
        if (s.breakpoints !== p.breakpoints || s.build !== p.build) sim({ type: 'setBreakpoints', pcs: enabledBreakpointPcs() });
      }),
    [],
  );

  // When execution stops somewhere in another file, bring that file to front.
  useEffect(
    () =>
      useSim.subscribe((s, p) => {
        if (s.revealSeq === p.revealSeq || !s.lastStop || s.lastStop.reason === 'load' || !s.state) return;
        const program = useWorkspace.getState().build?.program;
        const loc = pcToSource(program, s.state.pc);
        if (!loc) return;
        const ws = useWorkspace.getState();
        const doc = ws.docs.find((d) => sameFile(loc.file, d.path ?? d.name));
        if (doc) {
          if (doc.id !== ws.activeDocId) requestGoto(doc.id, loc.line);
        } else if (/[\\/]/.test(loc.file)) {
          void openPath(loc.file).then((ok) => ok && requestGoto(useWorkspace.getState().activeDocId!, loc.line));
        }
      }),
    [],
  );

  return (
    <div className={`app${maximized ? ' maximized' : ''}${focused ? '' : ' inactive'}`}>
      <IconDefs />
      <TitleBar maximized={maximized} />
      <MenuBar />
      <Toolbar />
      <DockLayout editor={<EditorArea />} render={renderPanel} />
      <StatusBar />
      <DialogHost />
      <ContextMenuHost />
      <TooltipHost />
    </div>
  );
}
