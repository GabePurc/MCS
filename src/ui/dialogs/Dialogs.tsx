import { useEffect, useState, type JSX, type ReactNode } from 'react';
import { CaptionGlyph, Icons } from '../icons';
import { closeDialog, useDialogs } from '../state/dialogs';
import { useSettings } from '../state/settings';
import { useSim } from '../state/sim';
import { defInclude, detectToolchain, instructionSet, pickFile, platform } from '../backend/api';
import type { InsnInfo, ToolchainInfo } from '../backend/types';
import { sim } from '../services/simClient';
import { hex } from '../format';

/** Aero-framed modal dialog with a Windows 7 TaskDialog-style button area. */
export function Dialog({ title, children, buttons, width = 460, gray }: { title: string; children: ReactNode; buttons?: ReactNode; width?: number; gray?: boolean }): JSX.Element {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && closeDialog();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && closeDialog()}>
      <div className="dialog" style={{ width }}>
        <div className="dialog-title">
          <Icons.App />
          <span className="grow">{title}</span>
          <div className="caption-buttons">
            <button className="caption-btn close" onClick={closeDialog} title="Close">
              <CaptionGlyph.Close />
            </button>
          </div>
        </div>
        <div className="dialog-client">
          <div className={`dialog-content${gray ? ' gray' : ''}`}>{children}</div>
          <div className="dialog-buttons">
            {buttons ?? (
              <button className="w7-btn default" onClick={closeDialog}>
                <span>OK</span>
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export function DialogHost(): JSX.Element | null {
  const open = useDialogs((s) => s.open);
  switch (open) {
    case 'about': return <AboutDialog />;
    case 'toolchain': return <ToolchainDialog />;
    case 'toolchainHelp': return <ToolchainHelpDialog />;
    case 'fuses': return <FusesDialog />;
    case 'supply': return <SupplyDialog />;
    case 'isa': return <IsaDialog />;
    case 'include': return <IncludeDialog />;
    default: return null;
  }
}

function AboutDialog(): JSX.Element {
  return (
    <Dialog title="About MCS" width={480}>
      <div style={{ display: 'flex', gap: 16 }}>
        <img src={new URL('../assets/app-icon-32.png', import.meta.url).href} width={48} height={48} alt="" />
        <div>
          <h2 className="dialog-main-instruction">MCS Microcontroller Simulator</h2>
          <p>Version 0.1.0</p>
          <p>Cycle-accurate microcontroller simulation with a built-in AVR assembler, avr-gcc integration and a source-level debugger. Simulation core written in Rust; UI rendered with Tauri.</p>
          <p className="dim">Fonts: Selawik (© Microsoft, SIL OFL 1.1), Cascadia Mono (© Microsoft, SIL OFL 1.1).</p>
        </div>
      </div>
    </Dialog>
  );
}

function ToolchainDialog(): JSX.Element {
  const s = useSettings();
  const [path, setPath] = useState(s.gccPath);
  const [opt, setOpt] = useState(s.optimize);
  const [flags, setFlags] = useState(s.extraFlags);
  const [font, setFont] = useState(s.editorFontSize);
  const [status, setStatus] = useState<ToolchainInfo | null | 'checking'>('checking');
  useEffect(() => {
    setStatus('checking');
    const t = setTimeout(() => detectToolchain(path || null).then(setStatus).catch(() => setStatus(null)), 250);
    return () => clearTimeout(t);
  }, [path]);
  const save = () => {
    s.set({ gccPath: path, optimize: opt, extraFlags: flags, editorFontSize: font });
    closeDialog();
  };
  return (
    <Dialog
      title="Toolchain Options"
      width={560}
      gray
      buttons={
        <>
          <button className="w7-btn default" onClick={save}><span>OK</span></button>
          <button className="w7-btn" onClick={closeDialog}><span>Cancel</span></button>
        </>
      }
    >
      <div className="w7-group">
        <span className="w7-group-title">AVR GCC (C and .S files)</span>
        <div className="form-grid">
          <span>avr-gcc path:</span>
          <div style={{ display: 'flex', gap: 6 }}>
            <input className="w7-input" style={{ flex: 1 }} placeholder="Auto-detect (PATH, Homebrew, Arduino, Microchip Studio...)" value={path} onChange={(e) => setPath(e.target.value)} />
            <button className="w7-btn small" onClick={async () => { const p = await pickFile('Locate avr-gcc', []); if (p) setPath(p); }}><span>Browse...</span></button>
          </div>
          <span>Status:</span>
          <span>
            {status === 'checking' ? 'Checking...' : status ? (
              <><Icons.Success size={13} /> {status.version}<br /><span className="dim mono" style={{ fontSize: 11 }}>{status.gcc}</span></>
            ) : (
              <><Icons.Warning size={13} /> avr-gcc not found - see Help &gt; C Toolchain Setup. Assembly files work without it.</>
            )}
          </span>
          <span>Optimization:</span>
          <select className="w7-select" value={opt} onChange={(e) => setOpt(e.target.value)} style={{ width: 220 }}>
            <option value="Os">-Os (size, recommended)</option>
            <option value="Og">-Og (debugging)</option>
            <option value="O0">-O0 (none)</option>
            <option value="O1">-O1</option>
            <option value="O2">-O2</option>
          </select>
          <span>Extra flags:</span>
          <input className="w7-input mono" value={flags} placeholder="-DF_CPU=1000000UL" onChange={(e) => setFlags(e.target.value)} />
        </div>
      </div>
      <div className="w7-group">
        <span className="w7-group-title">Editor</span>
        <div className="form-grid">
          <span>Font size:</span>
          <select className="w7-select" value={font} onChange={(e) => setFont(Number(e.target.value))} style={{ width: 90 }}>
            {[11, 12, 13, 14, 15, 16, 18].map((n) => <option key={n} value={n}>{n} px</option>)}
          </select>
        </div>
      </div>
    </Dialog>
  );
}

function ToolchainHelpDialog(): JSX.Element {
  const cmds: Record<string, [string, string][]> = {
    macos: [['Homebrew', 'brew tap osx-cross/avr && brew install avr-gcc@14'], ['Arduino IDE', 'Install the Arduino IDE and the "Arduino AVR Boards" package (its avr-gcc is detected automatically).']],
    windows: [['Microchip', 'Install the "AVR 8-bit Toolchain" from microchip.com (or Microchip Studio).'], ['Arduino IDE', 'Install the Arduino IDE; its bundled avr-gcc is detected automatically.']],
    linux: [['Debian / Ubuntu', 'sudo apt install gcc-avr avr-libc binutils-avr'], ['Fedora', 'sudo dnf install avr-gcc avr-libc avr-binutils'], ['Arch', 'sudo pacman -S avr-gcc avr-libc']],
  };
  return (
    <Dialog title="C Toolchain Setup" width={600}>
      <h2 className="dialog-main-instruction">Install avr-gcc to build C programs</h2>
      <p>Assembly (.asm) files are built by MCS itself. C and GNU assembler (.S) files are compiled with avr-gcc + avr-libc, which MCS finds on your PATH and in the usual install locations. You can also point to it in Tools &gt; Toolchain Options.</p>
      {(['macos', 'windows', 'linux'] as const).map((os) => (
        <div key={os} className="w7-group" style={{ background: os === platform ? '#f3f8fe' : undefined }}>
          <span className="w7-group-title" style={{ background: '#fff' }}>{{ macos: 'macOS', windows: 'Windows', linux: 'Linux' }[os]}{os === platform ? ' (this computer)' : ''}</span>
          {cmds[os].map(([k, v]) => (
            <div key={k} style={{ marginBottom: 4 }}>
              <b>{k}:</b> <span className="mono selectable">{v}</span>
            </div>
          ))}
        </div>
      ))}
    </Dialog>
  );
}

function FusesDialog(): JSX.Element {
  const spec = useSim((s) => s.spec);
  const st = useSim((s) => s.state);
  const [fuse, setFuse] = useState(st?.fuse ?? spec?.fuseDefault ?? 0xff);
  if (!spec) return <Dialog title="Fuses">No device loaded.</Dialog>;
  return (
    <Dialog
      title={`Fuses - ${spec.name}`}
      gray
      buttons={
        <>
          <span className="left dim">Applying power-cycles the MCU.</span>
          <button className="w7-btn default" onClick={() => { sim({ type: 'writeFuse', value: fuse }); closeDialog(); }}><span>Apply</span></button>
          <button className="w7-btn" onClick={closeDialog}><span>Cancel</span></button>
        </>
      }
    >
      <div className="w7-group">
        <span className="w7-group-title">Configuration byte = {hex(fuse)}</span>
        {spec.fuseBits.map((f) => (
          <label key={f.name} className="w7-check" style={{ display: 'flex', margin: '6px 0' }} data-tip={f.desc}>
            <input type="checkbox" checked={(fuse & f.mask) === 0} onChange={(e) => setFuse(e.target.checked ? fuse & ~f.mask : fuse | f.mask)} />
            <b>{f.name}</b>&nbsp;<span className="dim">{f.desc}</span>
          </label>
        ))}
        <p className="dim" style={{ marginBottom: 0 }}>Checked = programmed (bit value 0), like Atmel Studio's fuse view.</p>
      </div>
    </Dialog>
  );
}

function SupplyDialog(): JSX.Element {
  const st = useSim((s) => s.state);
  const vcc = useSettings((s) => s.vcc);
  const [ext, setExt] = useState('8000000');
  return (
    <Dialog title="Supply & Clock" gray>
      <div className="w7-group">
        <span className="w7-group-title">Supply voltage</span>
        <div className="supply-row">
          <input type="range" className="w7-slider" min={1.8} max={5.5} step={0.05} value={vcc} onChange={(e) => { const v = Number(e.target.value); useSettings.getState().set({ vcc: v }); sim({ type: 'setVcc', volts: v }); }} />
          <span className="mono">{vcc.toFixed(2)} V</span>
        </div>
        <p className="dim">Affects the ADC reference, the analog comparator and the VCC level monitor (VLM).</p>
      </div>
      <div className="w7-group">
        <span className="w7-group-title">External clock (CLKMSR = external)</span>
        <div className="supply-row">
          <input className="w7-input mono" value={ext} onChange={(e) => setExt(e.target.value)} style={{ width: 120 }} /> Hz
          <button className="w7-btn small" onClick={() => sim({ type: 'setExternalClock', hz: Number(ext) || 8e6 })}><span>Apply</span></button>
        </div>
        <p className="dim">Current CPU clock: {st ? `${(st.hz / 1e6).toFixed(3)} MHz` : '-'}</p>
      </div>
    </Dialog>
  );
}

function IsaDialog(): JSX.Element {
  const spec = useSim((s) => s.spec);
  const [rows, setRows] = useState<InsnInfo[]>([]);
  const [f, setF] = useState('');
  useEffect(() => {
    if (spec) instructionSet(spec.id).then(setRows).catch(() => {});
  }, [spec]);
  const shown = rows.filter((r) => !f || r.mnemonic.toLowerCase().includes(f.toLowerCase()));
  return (
    <Dialog title={`Instruction Set - ${spec?.name ?? ''} (${spec?.coreName ?? ''})`} width={640}>
      <input className="w7-input" placeholder="Filter mnemonics..." value={f} onChange={(e) => setF(e.target.value)} style={{ width: '100%', marginBottom: 8 }} autoFocus />
      <div style={{ maxHeight: '55vh', overflow: 'auto', border: '1px solid #d5dfe5' }}>
        <table className="grid-table">
          <thead>
            <tr><th>Mnemonic</th><th>Operands</th><th>Encoding</th><th>Cycles</th><th>Words</th></tr>
          </thead>
          <tbody>
            {shown.map((r, i) => (
              <tr key={i} className="row-hot">
                <td><b>{r.mnemonic}</b></td>
                <td className="mono">{r.operands}</td>
                <td className="mono dim">{r.encoding}</td>
                <td>{r.cycles}</td>
                <td>{r.words}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="dim">Branches take one extra cycle when taken; skips take 1 + the size of the skipped instruction. Aliases (CLR, LSL, TST, SER, BREQ, SEI, ...) are accepted by the assembler.</p>
    </Dialog>
  );
}

function IncludeDialog(): JSX.Element {
  const spec = useSim((s) => s.spec);
  const [inc, setInc] = useState<[string, string] | null>(null);
  useEffect(() => {
    if (spec) defInclude(spec.id).then(setInc).catch(() => {});
  }, [spec]);
  return (
    <Dialog
      title={inc ? inc[0] : 'Device definitions'}
      width={640}
      buttons={
        <>
          <button className="w7-btn" onClick={() => inc && void navigator.clipboard.writeText(inc[1])}><span>Copy</span></button>
          <button className="w7-btn default" onClick={closeDialog}><span>Close</span></button>
        </>
      }
    >
      <p>Use <span className="mono">.include "{inc?.[0]}"</span> in assembly sources. These definitions are generated from the device model:</p>
      <pre className="mono selectable include-view">{inc?.[1] ?? 'Loading...'}</pre>
    </Dialog>
  );
}
