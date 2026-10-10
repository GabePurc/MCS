import { useEffect, useState, type JSX, type ReactNode } from 'react';
import { CaptionGlyph, Icons } from '../icons';
import { closeDialog, useDialogs } from '../state/dialogs';
import { useSettings } from '../state/settings';
import { useSim } from '../state/sim';
import { detectToolchain, pickFile, platform } from '../backend/api';
import type { AvrDeviceSpec, FuseBitSpec, SpeedMode, ToolchainInfo } from '../backend/types';
import { sim } from '../services/simClient';
import { setSpeed, speedLabel } from '../services/commands';
import { formatHz, hex, parseHz } from '../format';
import { CustomDeviceDialog } from './CustomDeviceDialog';
import { APP_VERSION, checkForUpdates, installUpdate, useUpdates } from '../services/updater';
import { whatsNewSections } from '../services/whatsNew';

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
    case 'speed': return <SpeedDialog />;
    case 'update': return <UpdateDialog />;
    case 'customDevice': return <CustomDeviceDialog />;
    case 'whatsNew': return <WhatsNewDialog />;
    default: return null;
  }
}

/** `**bold**` spans of a changelog line. */
function inlineMd(text: string): ReactNode[] {
  return text.split(/\*\*(.+?)\*\*/g).map((part, i) => (i % 2 ? <b key={i}>{part}</b> : part));
}

/** Changelog bullets ("- " items whose continuation lines are indented) as a list. */
function ChangelogBody({ body }: { body: string }): JSX.Element {
  const items: string[] = [];
  for (const line of body.split('\n')) {
    if (/^\s*[-*] /.test(line)) items.push(line.replace(/^\s*[-*] /, ''));
    else if (line.trim() && items.length) items[items.length - 1] += ' ' + line.trim();
    else if (line.trim()) items.push(line.trim());
  }
  return <ul className="whats-new-list">{items.map((t, i) => <li key={i}>{inlineMd(t)}</li>)}</ul>;
}

function WhatsNewDialog(): JSX.Element {
  const sections = whatsNewSections();
  return (
    <Dialog title="What's New" width={600} gray buttons={<button className="w7-btn default" onClick={closeDialog}><span>OK</span></button>}>
      <h2 className="dialog-main-instruction">MCS has been updated to version {APP_VERSION}</h2>
      <div className="whats-new selectable">
        {sections.map((s) => (
          <div key={s.version} className="w7-group">
            <span className="w7-group-title">Version {s.version}</span>
            <ChangelogBody body={s.body} />
          </div>
        ))}
      </div>
    </Dialog>
  );
}

function AboutDialog(): JSX.Element {
  return (
    <Dialog title="About MCS" width={480}>
      <div style={{ display: 'flex', gap: 16 }}>
        <img src={new URL('../assets/app-icon-32.png', import.meta.url).href} width={48} height={48} alt="" />
        <div>
          <h2 className="dialog-main-instruction">MCS Microcontroller Simulator</h2>
          <p>Version {APP_VERSION}</p>
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

/** "(0000 external clock, 0010 internal RC, 1000-1111 crystal)" -> value labels. */
function fieldOptions(f: FuseBitSpec, spec: AvrDeviceSpec): string[] {
  const width = popcount8(f.mask);
  const n = 1 << width;
  const labels = Array.from({ length: n }, () => '');
  if (f.name === 'BOOTSZ' && spec.boot) {
    return labels.map((_, v) => {
      const words = spec.boot!.sizesWords[v];
      const start = spec.flashSize / 2 - words;
      return `${words} words (boot at 0x${(start * 2).toString(16).toUpperCase()})`;
    });
  }
  const inner = /\(([^)]*)\)/.exec(f.desc)?.[1] ?? '';
  for (const item of inner.split(/,\s*/)) {
    const m = /^([01]+)(?:-([01]+))?\s+(.*)$/.exec(item.trim());
    if (!m) continue;
    const a = parseInt(m[1], 2);
    const b = m[2] ? parseInt(m[2], 2) : a;
    for (let v = a; v <= b && v < n; v++) labels[v] = m[3];
  }
  return labels;
}

function popcount8(m: number): number {
  let c = 0;
  for (; m; m &= m - 1) c++;
  return c;
}

/** Common fuse settings per device family. */
function fusePresets(spec: AvrDeviceSpec): [string, number[]][] {
  const out: [string, number[]][] = [['Factory default', spec.fuses.map((f) => f.default)]];
  if (spec.id === 'atmega328p') out.push(['Arduino Uno (16 MHz crystal, boot loader, BOD 2.7 V)', [0xff, 0xde, 0xfd]]);
  if (spec.id === 'atmega168pa') out.push(['Arduino Diecimila (16 MHz crystal, boot loader)', [0xff, 0xdd, 0xf8]]);
  if (spec.peripheralSet === 'mega-x8' || spec.peripheralSet === 'mega-x4' || spec.peripheralSet === 'mega-x0') out.push(['Internal 8 MHz (no clock divider)', [0xe2, ...spec.fuses.slice(1).map((f) => f.default)]]);
  if (spec.id === 'atmega2560') out.push(['Arduino Mega 2560 (16 MHz crystal, boot loader)', [0xff, 0xd8, 0xfd]]);
  // 16 MHz crystal (CKSEL = 1111, SUT = 11, no clock divider), JTAG interface disabled, SPIEN programmed.
  if (spec.peripheralSet === 'mega-x4' || spec.peripheralSet === 'mega-x0') out.push(['16 MHz crystal (JTAG disabled)', [0xff, 0xd9, 0xff]]);
  if (spec.peripheralSet === 'mega-legacy') {
    // CKSEL = 0100 internal 8 MHz; 16 MHz crystal needs CKOPT programmed (CKSEL = 1111, SUT = 11,
    // BOD off): ATmega8 high 0xC9, ATmega16/32 high 0x89 (JTAGEN stays programmed).
    const high = spec.fuses[1]?.default ?? 0xff;
    out.push(['Internal 8 MHz RC', [0xe4, high]]);
    out.push(['16 MHz crystal (CKOPT programmed)', [0xff, high & ~0x10]]);
  }
  if (spec.peripheralSet === 'tiny13') out.push(['Internal 9.6 MHz (no clock divider)', [0x7a, ...spec.fuses.slice(1).map((f) => f.default)]]);
  if (spec.peripheralSet === 'tiny-x4') out.push(['Internal 8 MHz (no clock divider)', [0xe2, ...spec.fuses.slice(1).map((f) => f.default)]]);
  if (spec.peripheralSet === 'tiny-x313') out.push(['Internal 8 MHz (no clock divider)', [0xe4, ...spec.fuses.slice(1).map((f) => f.default)]]);
  if (spec.peripheralSet === 'tiny-x5') {
    out.push(['Internal 8 MHz (no clock divider)', [0xe2, 0xdf, 0xff]]);
    out.push(['16 MHz PLL clock (Digispark style)', [0xf1, 0xdd, 0xfe]]);
  }
  return out;
}

function FusesDialog(): JSX.Element {
  const spec = useSim((s) => (s.spec?.arch === 'avr' ? s.spec : null));
  const st = useSim((s) => s.state);
  const [fuses, setFuses] = useState<number[]>(() => st?.fuses.slice() ?? spec?.fuses.map((f) => f.default) ?? []);
  if (!spec) return <Dialog title="Fuses">No device loaded.</Dialog>;
  const setByte = (i: number, v: number) => setFuses((f) => f.map((x, k) => (k === i ? v & 0xff : x)));
  const apply = () => {
    fuses.forEach((v, i) => {
      if (v !== st?.fuses[i]) sim({ type: 'writeFuse', index: i, value: v });
    });
    closeDialog();
  };
  return (
    <Dialog
      title={`Fuses - ${spec.name}`}
      width={spec.fuses.length > 1 ? 620 : 460}
      gray
      buttons={
        <>
          <span className="left dim">Applying power-cycles the MCU.</span>
          <button className="w7-btn default" onClick={apply}><span>Apply</span></button>
          <button className="w7-btn" onClick={closeDialog}><span>Cancel</span></button>
        </>
      }
    >
      {fusePresets(spec).length > 1 && (
        <div className="supply-row" style={{ marginBottom: 8 }}>
          <span>Preset:</span>
          <select className="w7-select" value="" onChange={(e) => { const p = fusePresets(spec)[Number(e.target.value)]; if (p) setFuses(p[1].slice()); }}>
            <option value="">Choose...</option>
            {fusePresets(spec).map(([n], i) => <option key={n} value={i}>{n}</option>)}
          </select>
        </div>
      )}
      {spec.fuses.map((fb, i) => (
        <div key={fb.name} className="w7-group">
          <span className="w7-group-title">
            {fb.name} fuse ={' '}
            <input className="w7-input mono fuse-hex" value={hex(fuses[i] ?? 0xff)} onChange={(e) => { const v = parseInt(e.target.value.replace(/^0x/i, ''), 16); if (!Number.isNaN(v)) setByte(i, v); }} />
            {fuses[i] !== fb.default && <span className="dim"> (default {hex(fb.default)})</span>}
          </span>
          {fb.bits.map((f) => {
            const shift = Math.log2(f.mask & -f.mask);
            const value = ((fuses[i] ?? 0xff) & f.mask) >> shift;
            if (popcount8(f.mask) === 1) {
              return (
                <label key={f.name} className="w7-check fuse-row" data-tip={f.desc}>
                  <input type="checkbox" checked={value === 0} onChange={(e) => setByte(i, e.target.checked ? fuses[i] & ~f.mask : fuses[i] | f.mask)} />
                  <b>{f.name}</b>&nbsp;<span className="dim">{f.desc}</span>
                </label>
              );
            }
            const labels = fieldOptions(f, spec);
            const width = popcount8(f.mask);
            return (
              <div key={f.name} className="fuse-row" data-tip={f.desc}>
                <b>{f.name}</b>
                <select className="w7-select mono" value={value} onChange={(e) => setByte(i, (fuses[i] & ~f.mask) | ((Number(e.target.value) << shift) & f.mask))}>
                  {labels.map((l, v) => (
                    <option key={v} value={v}>{v.toString(2).padStart(width, '0')}{l ? ` - ${l}` : ''}</option>
                  ))}
                </select>
              </div>
            );
          })}
        </div>
      ))}
      <p className="dim" style={{ marginBottom: 0 }}>Checked = programmed (bit value 0), like Atmel Studio's fuse view. Clock-source fuses (CKSEL, CKDIV8) take effect at the power cycle; set the crystal / external frequency under Device &gt; Supply &amp; Clock.</p>
    </Dialog>
  );
}

const PRESCALERS = [0, 1, 2, 3, 4, 5, 6, 7, 8];

function SupplyDialog(): JSX.Element {
  const st = useSim((s) => s.state);
  const spec = useSim((s) => s.spec);
  // The clock source / prescaler registers are AVR-only; ARM parts just get the external clock (HSE) input.
  const avr = spec?.arch === 'avr' ? spec : null;
  const vcc = useSettings((s) => s.vcc);
  const msr = avr?.registers.find((r) => r.name === 'CLKMSR');
  const psr = avr?.registers.find((r) => r.name === 'CLKPSR');
  const clkpr = avr?.registers.find((r) => r.name === 'CLKPR');
  const curSource = msr && st ? st.data[msr.addr] & 3 : 0;
  const curPs = (psr ?? clkpr) && st ? st.data[(psr ?? clkpr)!.addr] & 0x0f : avr?.clock.defaultPrescaleLog2 ?? 0;
  const [source, setSource] = useState(curSource);
  const [ps, setPs] = useState(Math.min(curPs, 8));
  const [ext, setExt] = useState(formatHz(st?.extClockHz ?? 8e6));
  const extHz = parseHz(ext);
  const clki = spec?.pins.find((p) => p.functions.includes('CLKI'));
  const maxHz = spec ? speedGradeMax(spec.speedGrades, vcc) : 0;
  const resulting = (source === 0 ? avr?.clock.internalHz ?? 0 : source === 1 ? avr?.clock.slowHz ?? 0 : extHz || 0) / 2 ** ps;
  const sourceNames = [`Internal ${formatHz(avr?.clock.internalHz ?? 8e6)} RC oscillator`, `Internal ${formatHz(avr?.clock.slowHz ?? 128e3)} oscillator`, `External clock on CLKI${clki ? ` (${clki.name}, pin ${clki.number})` : ''}`];
  const apply = () => {
    if (extHz > 0 && spec?.arch !== 'riscv') sim({ type: 'setExternalClock', hz: extHz });
    if ((msr && psr) || clkpr) sim({ type: 'setClockConfig', source, prescaleLog2: ps });
  };
  return (
    <Dialog
      title="Supply & Clock"
      width={520}
      gray
      buttons={
        <>
          <span className="left dim">Applies immediately, even while running.</span>
          <button className="w7-btn default" onClick={() => { apply(); closeDialog(); }}><span>OK</span></button>
          <button className="w7-btn" onClick={apply} disabled={!(extHz > 0) && spec?.arch !== 'riscv'}><span>Apply</span></button>
          <button className="w7-btn" onClick={closeDialog}><span>Cancel</span></button>
        </>
      }
    >
      <div className="w7-group">
        <span className="w7-group-title">Supply voltage</span>
        <div className="supply-row">
          <input type="range" className="w7-slider" min={spec?.vccRange[0] ?? 1.8} max={spec?.vccRange[1] ?? 5.5} step={0.05} value={vcc} onChange={(e) => { const v = Number(e.target.value); useSettings.getState().set({ vcc: v }); sim({ type: 'setVcc', volts: v }); }} />
          <span className="mono">{vcc.toFixed(2)} V</span>
        </div>
        <p className="dim">{spec && spec.arch !== 'avr' ? 'Affects the ADC reference and the pin input thresholds.' : 'Affects the ADC reference, the analog comparator and the VCC level monitor (VLM).'} {spec && maxHz > 0 && <>Datasheet speed grade at this voltage: up to <b>{formatHz(maxHz)}</b>.</>}</p>
        {st && maxHz > 0 && st.hz > maxHz * 1.0001 && (
          <div className="hint warn"><Icons.Warning size={13} /> The CPU runs at {formatHz(st.hz)}, faster than the {formatHz(maxHz)} allowed at {vcc.toFixed(2)} V. A real chip may not run reliably.</div>
        )}
      </div>
      <div className="w7-group">
        <span className="w7-group-title">CPU clock</span>
        <p style={{ marginTop: 0 }}>Current CPU clock: <b className="mono">{st ? formatHz(st.hz) : '-'}</b>{msr && st && <span className="dim"> ({sourceNames[curSource].replace(/ \(.*\)$/, '')}, /{2 ** curPs})</span>}</p>
        {msr && psr ? (
          <div className="form-grid">
            <span>Source (CLKMSR):</span>
            <div>
              {sourceNames.map((n, i) => (
                <label key={i} className="w7-check" style={{ display: 'flex', margin: '2px 0' }}>
                  <input type="radio" checked={source === i} onChange={() => setSource(i)} /> {n}
                </label>
              ))}
            </div>
            <span>External clock:</span>
            <div className="supply-row">
              <input className="w7-input mono" value={ext} onChange={(e) => setExt(e.target.value)} style={{ width: 120 }} />
              <span className={extHz > 0 ? 'dim' : 'error-text'}>{extHz > 0 ? `= ${extHz.toLocaleString()} Hz` : 'e.g. 8 MHz, 32768 Hz, 16e6'}</span>
            </div>
            <span>Prescaler (CLKPSR):</span>
            <select className="w7-select" value={ps} onChange={(e) => setPs(Number(e.target.value))} style={{ width: 120 }}>
              {PRESCALERS.map((p) => <option key={p} value={p}>/{2 ** p}</option>)}
            </select>
            <span>Result:</span>
            <b className="mono">{formatHz(resulting)}</b>
          </div>
        ) : clkpr && avr ? (
          <ClassicClock spec={avr} ext={ext} setExt={setExt} extHz={extHz} ps={ps} setPs={setPs} />
        ) : spec?.arch === 'riscv' ? null : (
          <div className="supply-row">
            External clock: <input className="w7-input mono" value={ext} onChange={(e) => setExt(e.target.value)} style={{ width: 120 }} />
          </div>
        )}
        <p className="dim" style={{ marginBottom: 0 }}>
          {spec?.arch === 'riscv'
            ? 'The crystal is fixed at 40 MHz. The firmware selects the CPU clock (XTAL / 2 at reset, the 80 / 160 MHz PLL or RC_FAST) through the SYSTEM registers.'
            : avr === null
            ? 'The firmware configures the STM32 clock tree (HSI16 / HSE / PLL and the bus prescalers) through RCC. The external frequency above is the HSE crystal or clock input.'
            : msr
            ? 'On this chip the program selects the clock itself (CLKMSR/CLKPSR, protected by CCP). Apply writes those registers the way the debugger would; firmware that changes them later wins. The external frequency is used whenever the external source is selected.'
            : 'On this chip the clock source is chosen by the CKSEL fuses (changing them power-cycles the chip); the program can only change the prescaler (CLKPR). The external / crystal frequency is used when CKSEL selects an external clock or a crystal.'}
        </p>
      </div>
    </Dialog>
  );
}

/** Clock source of a fuse-configured AVR (CKSEL / CKDIV8 in the low fuse, CLKPR at run time). */
function ClassicClock({ spec, ext, setExt, extHz, ps, setPs }: { spec: AvrDeviceSpec; ext: string; setExt: (s: string) => void; extHz: number; ps: number; setPs: (n: number) => void }): JSX.Element {
  const st = useSim((s) => s.state);
  const fuses = st?.fuses ?? spec.fuses.map((f) => f.default);
  const low = fuses[0] ?? 0xff;
  const cksel = low & 0x0f;
  const ckselField = spec.fuses[0]?.bits.find((b) => b.name === 'CKSEL');
  const labels = ckselField ? fieldOptions(ckselField, spec) : [];
  const usesExt = /external|crystal/i.test(labels[cksel] ?? '') && !/32 kHz/.test(labels[cksel] ?? '');
  const setLow = (v: number) => sim({ type: 'writeFuse', index: 0, value: v & 0xff });
  return (
    <div className="form-grid">
      <span>Source (CKSEL fuses):</span>
      <select className="w7-select" value={cksel} onChange={(e) => setLow((low & 0xf0) | Number(e.target.value))} data-tip="Writes the low fuse and power-cycles the chip">
        {labels.map((l, v) => <option key={v} value={v}>{v.toString(2).padStart(4, '0')}{l ? ` - ${l}` : ' - reserved'}</option>)}
      </select>
      <span>CKDIV8 fuse:</span>
      <label className="w7-check" data-tip="Programmed: the prescaler starts at /8 after reset (writes the low fuse and power-cycles)">
        <input type="checkbox" checked={(low & 0x80) === 0} onChange={(e) => setLow(e.target.checked ? low & 0x7f : low | 0x80)} /> Divide by 8 at reset (factory setting)
      </label>
      <span>{usesExt ? 'Crystal / external clock:' : 'External clock:'}</span>
      <div className="supply-row">
        <input className="w7-input mono" value={ext} onChange={(e) => setExt(e.target.value)} style={{ width: 120 }} />
        <span className={extHz > 0 ? 'dim' : 'error-text'}>{extHz > 0 ? (usesExt ? 'in use' : 'used when CKSEL selects it') : 'e.g. 16 MHz'}</span>
      </div>
      <span>Prescaler (CLKPR):</span>
      <select className="w7-select" value={ps} onChange={(e) => setPs(Number(e.target.value))} style={{ width: 120 }}>
        {PRESCALERS.map((p) => <option key={p} value={p}>/{2 ** p}</option>)}
      </select>
    </div>
  );
}

function speedGradeMax(grades: [number, number][], vcc: number): number {
  return grades.filter(([, v]) => vcc + 1e-9 >= v).reduce((m, [hz]) => Math.max(m, hz), 0);
}

const LOG_MIN = 0; // 1 Hz
const LOG_MAX = 8; // 100 MHz

function SpeedDialog(): JSX.Element {
  const s = useSettings();
  const mcuHz = useSim((x) => x.state?.hz ?? 1e6);
  const [mode, setMode] = useState<SpeedMode>(s.speedMode);
  const [hz, setHz] = useState(s.speedMode === 'clock' ? s.speedFactor : Math.min(mcuHz, 1e8));
  const [hzText, setHzText] = useState(formatHz(hz));
  const [factor, setFactor] = useState(s.speedMode === 'realtime' ? s.speedFactor : 1);
  const setHzBoth = (v: number) => {
    setHz(v);
    setHzText(formatHz(v));
  };
  const apply = () => {
    if (mode === 'clock') setSpeed('clock', Math.max(0.01, hz));
    else if (mode === 'realtime') setSpeed('realtime', factor);
    else setSpeed('max', 1);
  };
  const presets = [1, 10, 100, 1e3, 1e4, 1e5, 1e6];
  return (
    <Dialog
      title="Simulation Speed"
      width={540}
      gray
      buttons={
        <>
          <span className="left dim">Now: {speedLabel(s.speedMode, s.speedFactor)}</span>
          <button className="w7-btn default" onClick={() => { apply(); closeDialog(); }}><span>OK</span></button>
          <button className="w7-btn" onClick={apply}><span>Apply</span></button>
          <button className="w7-btn" onClick={closeDialog}><span>Cancel</span></button>
        </>
      }
    >
      <div className="w7-group">
        <label className="w7-check speed-choice">
          <input type="radio" checked={mode === 'clock'} onChange={() => setMode('clock')} />
          <span><b>Fixed CPU speed</b> - run exactly this many clock cycles per second, whatever the chip's own clock is. Slow it down to 1 Hz to watch every instruction in the Chip View.</span>
        </label>
        <div className={`speed-row${mode === 'clock' ? '' : ' disabled'}`}>
          <input type="range" className="w7-slider grow" min={LOG_MIN} max={LOG_MAX} step={0.01} value={Math.log10(Math.max(1, hz))} onChange={(e) => { setMode('clock'); setHzBoth(Math.round(10 ** Number(e.target.value) * 100) / 100); }} />
          <input className="w7-input mono" style={{ width: 100 }} value={hzText} onChange={(e) => { setMode('clock'); setHzText(e.target.value); const v = parseHz(e.target.value); if (v > 0) setHz(v); }} />
        </div>
        <div className={`speed-presets${mode === 'clock' ? '' : ' disabled'}`}>
          {presets.map((p) => (
            <button key={p} className={`seg-btn${mode === 'clock' && hz === p ? ' on' : ''}`} onClick={() => { setMode('clock'); setHzBoth(p); }}>{formatHz(p)}</button>
          ))}
          <button className={`seg-btn${mode === 'clock' && hz === mcuHz ? ' on' : ''}`} onClick={() => { setMode('clock'); setHzBoth(mcuHz); }} data-tip="The MCU's current clock">MCU ({formatHz(mcuHz)})</button>
        </div>
      </div>
      <div className="w7-group">
        <label className="w7-check speed-choice">
          <input type="radio" checked={mode === 'realtime'} onChange={() => setMode('realtime')} />
          <span><b>Relative to real time</b> - 1x runs at the chip's actual clock ({formatHz(mcuHz)} now) and follows it when the program changes the clock.</span>
        </label>
        <div className={`speed-row${mode === 'realtime' ? '' : ' disabled'}`}>
          <input type="range" className="w7-slider grow" min={-3} max={3} step={0.01} value={Math.log10(factor)} onChange={(e) => { setMode('realtime'); setFactor(+(10 ** Number(e.target.value)).toPrecision(3)); }} />
          <span className="mono" style={{ width: 100, textAlign: 'right' }}>{speedLabel('realtime', factor)}</span>
        </div>
        <div className={`speed-presets${mode === 'realtime' ? '' : ' disabled'}`}>
          {[0.001, 0.01, 0.1, 1, 10, 100].map((f) => (
            <button key={f} className={`seg-btn${mode === 'realtime' && factor === f ? ' on' : ''}`} onClick={() => { setMode('realtime'); setFactor(f); }}>{speedLabel('realtime', f)}</button>
          ))}
        </div>
      </div>
      <div className="w7-group">
        <label className="w7-check speed-choice">
          <input type="radio" checked={mode === 'max'} onChange={() => setMode('max')} />
          <span><b>Maximum</b> - as fast as this computer can simulate (typically 100-200 million instructions per second, far beyond any real AVR).</span>
        </label>
      </div>
    </Dialog>
  );
}

function UpdateDialog(): JSX.Element {
  const st = useUpdates((s) => s.state);
  const auto = useSettings((s) => s.autoUpdateCheck);
  const pct = st.kind === 'downloading' && st.total > 0 ? Math.min(100, (st.done / st.total) * 100) : null;
  return (
    <Dialog
      title="Check for Updates"
      width={520}
      gray
      buttons={
        <>
          <label className="w7-check left">
            <input type="checkbox" checked={auto} onChange={(e) => useSettings.getState().set({ autoUpdateCheck: e.target.checked })} /> Check automatically at start-up
          </label>
          {st.kind === 'available' && <button className="w7-btn default" onClick={() => void installUpdate()}><span>Install and Restart</span></button>}
          {(st.kind === 'none' || st.kind === 'error') && <button className="w7-btn" onClick={() => void checkForUpdates(true)}><span>Check Again</span></button>}
          <button className="w7-btn" disabled={st.kind === 'downloading' || st.kind === 'installed'} onClick={closeDialog}><span>{st.kind === 'available' ? 'Later' : 'Close'}</span></button>
        </>
      }
    >
      <div className="w7-group">
        <span className="w7-group-title">MCS Microcontroller Simulator {APP_VERSION}</span>
        {st.kind === 'checking' && <p>Looking for a newer version...</p>}
        {st.kind === 'idle' && <p>Press "Check Again" to look for a newer version.</p>}
        {st.kind === 'none' && <p><Icons.Success size={13} /> You have the latest version.</p>}
        {st.kind === 'error' && (
          <p><Icons.Warning size={13} /> Could not check for updates: <span className="dim">{st.message}</span><br />Check the internet connection, or download the latest release from github.com/GabePurc/MCS.</p>
        )}
        {st.kind === 'available' && (
          <>
            <p><b>Version {st.version}</b> is available{st.date ? ` (released ${st.date.slice(0, 10)})` : ''}. It downloads and installs itself, then MCS restarts. Open files are kept; you are asked to save unsaved changes first.</p>
            {st.notes && <pre className="update-notes selectable">{st.notes}</pre>}
          </>
        )}
        {st.kind === 'downloading' && (
          <>
            <p>Downloading version {st.version}...{st.total > 0 ? ` ${(st.done / 1048576).toFixed(1)} of ${(st.total / 1048576).toFixed(1)} MB` : ''}</p>
            <div className={`w7-progress${pct === null ? ' marquee' : ''}`}><div style={{ width: `${pct ?? 100}%` }} /></div>
          </>
        )}
        {st.kind === 'installed' && <p><Icons.Success size={13} /> Version {st.version} is installed. Restarting...</p>}
        {st.kind === 'checking' && <div className="w7-progress marquee"><div /></div>}
      </div>
    </Dialog>
  );
}
