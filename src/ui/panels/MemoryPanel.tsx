import { useMemo, useRef, useState, type JSX } from 'react';
import { avrCore } from '../backend/types';
import { useSim } from '../state/sim';
import { useWorkspace } from '../state/workspace';
import { sim } from '../services/simClient';
import { hex, hexRaw, parseNumber } from '../format';
import { EmptyHint } from './common';

type Space = 'data' | 'flash' | 'eeprom' | 'nvm';
const ROW_H = 18;
let savedSpace: Space = 'data';
let savedCols = 16;

/** Virtualized hex viewer/editor for data space and program memory. */
export function MemoryPanel(): JSX.Element {
  const spec = useSim((s) => s.spec);
  const st = useSim((s) => s.state);
  const base = useSim((s) => s.baseline);
  const flash = useSim((s) => s.flash);
  const eeprom = useSim((s) => s.eeprom);
  const build = useWorkspace((s) => s.build);
  const [space, setSpaceState] = useState<Space>(savedSpace);
  const [cols, setColsState] = useState(savedCols);
  const [scroll, setScroll] = useState(0);
  const [height, setHeight] = useState(300);
  const [editing, setEditing] = useState<number | null>(null);
  const [gotoText, setGotoText] = useState('');
  const scroller = useRef<HTMLDivElement>(null);
  const setSpace = (s: Space) => {
    savedSpace = s;
    setSpaceState(s);
  };
  const setCols = (c: number) => {
    savedCols = c;
    setColsState(c);
  };

  const names = useMemo(() => {
    const m = new Map<number, string>();
    if (!spec) return m;
    for (const r of spec.registers) m.set(r.addr, r.name);
    for (const s of build?.program.symbols ?? []) if (s.space === 'data') m.set(s.address, s.name);
    return m;
  }, [spec, build]);

  if (!spec || !st) return <EmptyHint>No device loaded.</EmptyHint>;

  if (space === 'nvm') {
    const rows: [string, string, string][] = [
      ['Signature', spec.signature.map((b) => hexRaw(b)).join(' '), `${spec.name} device ID`],
      ...spec.fuses.map((fb, i): [string, string, string] => [
        `${fb.name} fuse`,
        hex(st.fuses[i] ?? 0xff),
        fb.bits.map((f) => `${f.name}=${(((st.fuses[i] ?? 0xff) & f.mask) >> Math.log2(f.mask & -f.mask)).toString(2).padStart(Math.round(Math.log2((f.mask >> Math.log2(f.mask & -f.mask)) + 1)), '0')}`).join('  '),
      ]),
      ['Lock bits', hex(st.lock), 'NVLB'],
      ['Calibration', hex(spec.calibration), 'Factory OSCCAL value'],
    ];
    return (
      <div className="panel">
        <MemToolbar space={space} setSpace={setSpace} cols={cols} setCols={setCols} gotoText={gotoText} setGotoText={setGotoText} onGoto={() => {}} eeprom={spec.eepromSize > 0} />
        <div className="panel-scroll">
          <table className="grid-table">
            <thead>
              <tr><th>Location</th><th>Value</th><th>Meaning</th></tr>
            </thead>
            <tbody>
              {rows.map(([a, b, c]) => (
                <tr key={a} className="row-hot"><td>{a}</td><td className="mono">{b}</td><td className="dim">{c}</td></tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    );
  }

  const space2: Space = space === 'eeprom' && spec.eepromSize === 0 ? 'data' : space;
  const bytes = space2 === 'data' ? st.data : space2 === 'eeprom' ? eeprom ?? new Uint8Array(spec.eepromSize).fill(0xff) : flash ?? new Uint8Array(spec.flashSize).fill(0xff);
  const prev = space === 'data' ? base?.data : undefined;
  const total = bytes.length;
  const rowsN = Math.ceil(total / cols);
  const first = Math.max(0, Math.floor(scroll / ROW_H) - 2);
  const last = Math.min(rowsN, first + Math.ceil(height / ROW_H) + 4);
  const addrDigits = total > 0x10000 ? 6 : 4;
  const pcByte = st.pcBytes;
  const sp = avrCore(st).sp;
  const markClass = (a: number) => {
    if (space === 'data' && a === sp) return ' mark-sp';
    if (space === 'data' && a > sp && a < spec.sramStart + spec.sramSize) return ' mark-stack';
    if (space === 'flash' && (a === pcByte || a === pcByte + 1)) return ' mark-pc';
    return '';
  };
  const tipFor = (a: number) => {
    const n = space === 'data' ? names.get(a) : undefined;
    const region = space === 'flash' ? 'Flash' : space === 'eeprom' ? 'EEPROM' : a < spec.sramStart ? 'I/O' : 'SRAM';
    return `${region} ${hex(a, addrDigits)}${n ? ` - ${n}` : ''} = ${hex(bytes[a])} (${bytes[a]})${a === sp && space === 'data' ? '\n<- SP' : ''}\nDouble-click to edit`;
  };
  const commit = (a: number, v: number) => {
    if (space === 'data') sim({ type: 'writeData', addr: a, value: v });
    else if (space === 'eeprom') sim({ type: 'writeEeprom', addr: a, value: v });
    else sim({ type: 'writeFlash', addr: a, value: v });
  };
  const goto = () => {
    const a = parseNumber(gotoText);
    if (Number.isNaN(a) || !scroller.current) return;
    scroller.current.scrollTop = Math.floor(a / cols) * ROW_H;
  };

  return (
    <div className="panel">
      <MemToolbar space={space2} setSpace={setSpace} cols={cols} setCols={setCols} gotoText={gotoText} setGotoText={setGotoText} onGoto={goto} eeprom={spec.eepromSize > 0} />
      <div className="hex-header mono">
        <span className="hex-addr">Address</span>
        {Array.from({ length: cols }, (_, i) => (
          <span key={i} className="hex-byte dim">{hexRaw(i, 1)}</span>
        ))}
        <span className="hex-ascii dim">ASCII</span>
      </div>
      <div
        className="panel-scroll hex-scroll mono"
        ref={(el) => {
          scroller.current = el;
          if (el && el.clientHeight !== height) setHeight(el.clientHeight);
        }}
        onScroll={(e) => setScroll(e.currentTarget.scrollTop)}
      >
        <div style={{ height: rowsN * ROW_H, position: 'relative' }}>
          {Array.from({ length: last - first }, (_, k) => {
            const row = first + k;
            const start = row * cols;
            const cells = [];
            let ascii = '';
            for (let i = 0; i < cols; i++) {
              const a = start + i;
              if (a >= total) break;
              const v = bytes[a];
              ascii += v >= 0x20 && v < 0x7f ? String.fromCharCode(v) : '.';
              const ch = prev !== undefined && prev[a] !== v;
              cells.push(
                editing === a ? (
                  <input
                    key={a}
                    className="hex-edit"
                    autoFocus
                    maxLength={2}
                    defaultValue={hexRaw(v)}
                    onFocus={(e) => e.currentTarget.select()}
                    onBlur={() => setEditing(null)}
                    onKeyDown={(e) => {
                      if (e.key === 'Escape') setEditing(null);
                      if (e.key === 'Enter' || e.key === 'Tab') {
                        const nv = parseInt(e.currentTarget.value, 16);
                        if (!Number.isNaN(nv) && nv >= 0 && nv <= 255) commit(a, nv);
                        e.preventDefault();
                        setEditing(e.key === 'Tab' && a + 1 < total ? a + 1 : null);
                      }
                    }}
                  />
                ) : (
                  <span key={a} className={`hex-byte${ch ? ' changed' : ''}${a < spec.sramStart && space === 'data' ? ' io' : ''}${markClass(a)}`} data-tip={tipFor(a)} onDoubleClick={() => setEditing(a)}>
                    {hexRaw(v)}
                  </span>
                ),
              );
            }
            return (
              <div key={row} className="hex-row" style={{ top: row * ROW_H }}>
                <span className="hex-addr">{hexRaw(start, addrDigits)}</span>
                {cells}
                <span className="hex-ascii">{ascii}</span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function MemToolbar(p: { space: Space; setSpace: (s: Space) => void; cols: number; setCols: (c: number) => void; gotoText: string; setGotoText: (s: string) => void; onGoto: () => void; eeprom: boolean }): JSX.Element {
  return (
    <div className="panel-toolbar">
      <span>Memory:</span>
      <select className="w7-select" value={p.space} onChange={(e) => p.setSpace(e.target.value as Space)}>
        <option value="data">data (I/O + SRAM)</option>
        <option value="flash">prog (Flash)</option>
        {p.eeprom && <option value="eeprom">eeprom (EEPROM)</option>}
        <option value="nvm">fuses, lock, signature</option>
      </select>
      {p.space !== 'nvm' && (
        <>
          <span>Columns:</span>
          <select className="w7-select" value={p.cols} onChange={(e) => p.setCols(Number(e.target.value))}>
            {[4, 8, 16, 32].map((c) => <option key={c} value={c}>{c}</option>)}
          </select>
          <span>Go to:</span>
          <input className="w7-input mono" style={{ width: 80 }} placeholder="0x0040" value={p.gotoText} onChange={(e) => p.setGotoText(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && p.onGoto()} />
        </>
      )}
    </div>
  );
}
