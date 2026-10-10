import type { JSX } from 'react';
import { useWorkspace } from '../state/workspace';
import { closeDocument, newFile, openExample, openFileDialog, openPath } from '../services/files';
import { importHexOrElf } from '../services/build';
import { useSettings } from '../state/settings';
import { SHOWCASE } from '../services/examples';
import { baseName } from '../services/debugInfo';
import { Icons } from '../icons';
import { useDevices } from '../state/devices';
import { SourceEditor } from './SourceEditor';
import { SymbolSidebar } from './SymbolSidebar';
import { openContextMenu } from '../controls/Menu';
import { runCommand } from '../services/commands';

/** Document tab strip + editor (or the start page when nothing is open). */
export function EditorArea(): JSX.Element {
  const docs = useWorkspace((s) => s.docs);
  const activeId = useWorkspace((s) => s.activeDocId);
  const active = docs.find((d) => d.id === activeId);
  const symbolView = useSettings((s) => s.symbolView);
  return (
    <div className="dock-group" style={{ flex: 1 }}>
      <div className="dock-tabs doc-tabs">
        <div className={`dock-tab${!active ? ' active' : ''}`} onMouseDown={() => useWorkspace.setState({ activeDocId: null })}>
          <Icons.App size={14} />
          Start Page
        </div>
        {docs.map((d) => (
          <div
            key={d.id}
            className={`dock-tab${d.id === activeId ? ' active' : ''}`}
            data-tip={d.path ?? `${d.name} (not saved)`}
            onMouseDown={(e) => {
              if (e.button === 1) void closeDocument(d.id);
              else useWorkspace.setState({ activeDocId: d.id });
            }}
            onContextMenu={(e) =>
              openContextMenu(e, [
                { kind: 'action', label: 'Save', icon: 'Save', run: () => runCommand('file.save') },
                { kind: 'action', label: 'Close', run: () => void closeDocument(d.id) },
                { kind: 'action', label: 'Close All But This', run: () => docs.filter((x) => x.id !== d.id).forEach((x) => void closeDocument(x.id)) },
              ])
            }
          >
            {d.language === 'c' ? <span className="lang-badge c">C</span> : d.language === 'mc' ? <span className="lang-badge mc">MC</span> : <span className="lang-badge asm">ASM</span>}
            {d.name}
            {d.dirty && <span className="dirty">*</span>}
            <span className="tab-close" onMouseDown={(e) => e.stopPropagation()} onClick={() => void closeDocument(d.id)}>
              <Icons.Close size={7} />
            </span>
          </div>
        ))}
        <div className="tabs-spacer" />
      </div>
      <div className="dock-body editor-body">
        {active && symbolView && <SymbolSidebar />}
        {active ? <div className="editor-main"><SourceEditor doc={active} /></div> : <StartPage />}
      </div>
    </div>
  );
}

function StartPage(): JSX.Element {
  const recent = useSettings((s) => s.recentFiles);
  const device = useSettings((s) => s.deviceId);
  return (
    <div className="start-page">
      <div className="start-hero">
        <h1>MCS Microcontroller Simulator</h1>
        <p>Cycle-accurate {useDevices.getState().devices.find((d) => d.id === device)?.name ?? device} simulation - write assembly, C or machine code, build, then step through your code while watching registers, memory, pins and waveforms.</p>
      </div>
      <div className="start-columns">
        <div>
          <h2>Start</h2>
          <a className="start-link" onClick={() => newFile('asm')}><Icons.NewFile /> <span>New assembly file<span className="desc">Built-in avrasm2-compatible assembler</span></span></a>
          <a className="start-link" onClick={() => newFile('c')}><Icons.NewFile /> <span>New C file<span className="desc">Compiled with avr-gcc</span></span></a>
          <a className="start-link" onClick={() => newFile('mc')}><Icons.MachineCode /> <span>New machine code file<span className="desc">Write raw instruction words (hex or binary)</span></span></a>
          <a className="start-link" onClick={() => void openFileDialog()}><Icons.Open /> <span>Open file...<span className="desc">.asm, .S, .c, .h, .mc</span></span></a>
          <a className="start-link" onClick={() => void importHexOrElf()}><Icons.Import /> <span>Import HEX / ELF...<span className="desc">Run a program built elsewhere</span></span></a>
        </div>
        <div>
          <h2>Examples</h2>
          {SHOWCASE.map((e) => (
            <a key={e.name} className="start-link" onClick={() => openExample(e.name)}>
              <Icons.Disasm />
              <span>{e.title}{e.device && <span className="dev-badge">{e.device}</span>}<span className="desc">{e.description}</span></span>
            </a>
          ))}
        </div>
        <div>
          <h2>Recent files</h2>
          {recent.length === 0 && <div className="dim" style={{ padding: 6 }}>No recent files.</div>}
          {recent.map((p) => (
            <a key={p} className="start-link" onClick={() => void openPath(p)} data-tip={p}>
              <Icons.Open /> <span>{baseName(p)}<span className="desc">{p}</span></span>
            </a>
          ))}
          <h2 style={{ marginTop: 18 }}>Getting started</h2>
          <ol className="start-steps">
            <li>Open an example, e.g. <i>Blink (assembly)</i>.</li>
            <li>Press <b>F7</b> to build, <b>F5</b> to run, <b>F10/F11</b> to step.</li>
            <li>Click the left margin (or <b>F9</b>) to set breakpoints.</li>
            <li>Drive input pins in <i>Pins &amp; Stimulus</i> (levels, push button, signal generator); watch outputs in <i>Waveform</i>.</li>
            <li>Open <i>View &gt; Chip View (3D)</i> and set the speed to 1 Hz to watch the program run inside the chip.</li>
          </ol>
        </div>
      </div>
    </div>
  );
}
