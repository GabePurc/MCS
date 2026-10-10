import { useEffect, useMemo, useRef, useState, type JSX } from 'react';
import { useSim } from '../state/sim';
import { useWorkspace } from '../state/workspace';
import { sim, watchRam } from '../services/simClient';
import { hex, hexRaw, parseNumber } from '../format';
import { EmptyHint } from './common';

/** `ram<k>` = extra RAM block k (`extraRam[k - 1]`, ARM and RISC-V). */
type Space = 'data' | 'flash' | 'eeprom' | 'nvm' | `ram${number}`;
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

  // Extra RAM block shown (memory-mapped devices, 1-based), 0 when the view shows another memory.
  const nExtra = spec && spec.arch !== 'avr' ? spec.extraRam.length : 0;
  const xi = space.startsWith('ram') && Number(space.slice(3)) <= nExtra ? Number(space.slice(3)) : 0;
  useEffect(() => {
    if (!xi) return;
    watchRam(xi);
    return () => watchRam(0);
  }, [xi, spec?.id]);

  const names = useMemo(() => {
    const m = new Map<number, string>();
    if (!spec) return m;
    for (const r of spec.registers) m.set(r.addr, r.name);
    for (const s of build?.program.symbols ?? []) if (s.space === 'data') m.set(s.address, s.name);
    return m;
  }, [spec, build]);

  if (!spec || !st) return <EmptyHint>No device loaded.</EmptyHint>;

  const avr = spec.arch === 'avr' ? spec : null;
  // Devices whose memories sit at their bus addresses (ARM, RISC-V).
  const arm = spec.arch !== 'avr' ? spec : null;
  const riscv = spec.arch === 'riscv' ? spec : null;
  const eepromSize = avr?.eepromSize ?? 0;

  if (space === 'nvm' && avr) {
    const rows: [string, string, string][] = [
      ['Signature', avr.signature.map((b) => hexRaw(b)).join(' '), `${spec.name} device ID`],
      ...avr.fuses.map((fb, i): [string, string, string] => [
        `${fb.name} fuse`,
        hex(st.fuses[i] ?? 0xff),
        fb.bits.map((f) => `${f.name}=${(((st.fuses[i] ?? 0xff) & f.mask) >> Math.log2(f.mask & -f.mask)).toString(2).padStart(Math.round(Math.log2((f.mask >> Math.log2(f.mask & -f.mask)) + 1)), '0')}`).join('  '),
      ]),
      ['Lock bits', hex(st.lock), 'NVLB'],
      ['Calibration', hex(avr.calibration), 'Factory OSCCAL value'],
    ];
    return (
      <div className="panel">
        <MemToolbar space={space} setSpace={setSpace} cols={cols} setCols={setCols} gotoText={gotoText} setGotoText={setGotoText} onGoto={() => {}} eeprom={eepromSize > 0} arm={!!arm} />
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

  const space2: Space = (space === 'eeprom' && eepromSize === 0) || (space === 'nvm' && !avr) || (space.startsWith('ram') && !xi) ? 'data' : space;
  const block = arm && xi ? arm.extraRam[xi - 1] : null;
  const extra = block && st.ramExtra?.index === xi ? st.ramExtra.data : null;
  const bytes = block ? extra ?? new Uint8Array(0) : space2 === 'data' ? st.data : space2 === 'eeprom' ? eeprom ?? new Uint8Array(eepromSize).fill(0xff) : flash ?? new Uint8Array(spec.flashSize).fill(0xff);
  // Bus address of byte 0 of the viewed memory (ARM memories sit at their bus addresses).
  const addrBase = block ? block.base : arm ? (space2 === 'flash' ? arm.flashBase : arm.sramBase) : 0;
  const sramStart = avr ? avr.sramStart : 0;
  const sramEnd = block ? block.base + bytes.length : arm ? arm.sramBase + st.data.length : avr ? avr.sramStart + avr.sramSize : 0;
  const prev = block ? (base?.ramExtra?.index === xi && base.ramExtra.data.length === bytes.length ? base.ramExtra.data : undefined) : space === 'data' && base?.data.length === st.data.length ? base?.data : undefined;
  const isRam = space2 === 'data' || !!block;
  const total = bytes.length;
  const rowsN = Math.ceil(total / cols);
  const first = Math.max(0, Math.floor(scroll / ROW_H) - 2);
  const last = Math.min(rowsN, first + Math.ceil(height / ROW_H) + 4);
  const addrDigits = arm ? 8 : total > 0x10000 ? 6 : 4;
  const pcByte = st.pcBytes;
  // Bus address of the stack pointer (AVR: data space address; ARM: active SP).
  const sp = st.core.arch === 'arm' ? st.core.r[13] : st.core.arch === 'riscv' ? st.core.x[2] : st.core.sp;
  const markClass = (a: number) => {
    if (isRam && a === sp) return ' mark-sp';
    if (isRam && a > sp && sp >= addrBase && a < sramEnd) return ' mark-stack';
    if (space2 === 'flash' && (a === pcByte || a === pcByte + 1)) return ' mark-pc';
    return '';
  };
  const tipFor = (i: number) => {
    const a = i + addrBase;
    const n = isRam ? names.get(a) : undefined;
    const ccm = spec.arch === 'arm' ? spec.ccmSram : null;
    const region = block ? block.name : space2 === 'flash' ? (riscv ? 'Flash (IROM)' : 'Flash') : space2 === 'eeprom' ? 'EEPROM' : riscv ? 'SRAM1 (DRAM)' : arm ? (ccm && a >= ccm.aliasBase ? 'CCM SRAM' : 'SRAM') : a < sramStart ? 'I/O' : 'SRAM';
    return `${region} ${hex(a, addrDigits)}${n ? ` - ${n}` : ''} = ${hex(bytes[i])} (${bytes[i]})${riscv && space2 === 'data' && !block ? `\nIRAM alias ${hex(a - riscv.sramBase + riscv.iramBase, 8)}` : ''}${a === sp && isRam ? '\n<- SP' : ''}\nDouble-click to edit`;
  };
  const commit = (i: number, v: number) => {
    const a = i + addrBase;
    if (block) sim({ type: 'writeMem', addr: a, size: 1, value: v });
    else if (space2 === 'data') sim({ type: 'writeData', addr: a, value: v });
    else if (space2 === 'eeprom') sim({ type: 'writeEeprom', addr: a, value: v });
    else sim({ type: 'writeFlash', addr: a, value: v });
  };
  const goto = () => {
    const a = parseNumber(gotoText) - addrBase;
    if (Number.isNaN(a) || a < 0 || !scroller.current) return;
    scroller.current.scrollTop = Math.floor(a / cols) * ROW_H;
  };

  return (
    <div className="panel">
      <MemToolbar space={space2} setSpace={setSpace} cols={cols} setCols={setCols} gotoText={gotoText} setGotoText={setGotoText} onGoto={goto} eeprom={eepromSize > 0} arm={!!arm} extraRam={arm?.extraRam} labels={riscv ? ['DRAM / IRAM (SRAM1)', 'Flash (IROM)'] : undefined} note={riscv && space2 === 'data' ? `IRAM alias: ${hex(riscv.iramBase, 8)}` : riscv && space2 === 'flash' ? `Read-only data view (DROM) at ${hex(riscv.dromBase, 8)}` : undefined} placeholder={riscv ? hex(riscv.sramBase, 8) : undefined} />
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
                  <span key={a} className={`hex-byte${ch ? ' changed' : ''}${a < sramStart && space2 === 'data' ? ' io' : ''}${markClass(a + addrBase)}`} data-tip={tipFor(a)} onDoubleClick={() => setEditing(a)}>
                    {hexRaw(v)}
                  </span>
                ),
              );
            }
            return (
              <div key={row} className="hex-row" style={{ top: row * ROW_H }}>
                <span className="hex-addr">{hexRaw(start + addrBase, addrDigits)}</span>
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

function MemToolbar(p: { arm: boolean; space: Space; setSpace: (s: Space) => void; cols: number; setCols: (c: number) => void; gotoText: string; setGotoText: (s: string) => void; onGoto: () => void; eeprom: boolean; extraRam?: { name: string }[]; labels?: [string, string]; note?: string; placeholder?: string }): JSX.Element {
  return (
    <div className="panel-toolbar">
      <span>Memory:</span>
      <select className="w7-select" value={p.space} onChange={(e) => p.setSpace(e.target.value as Space)}>
        <option value="data">{p.labels?.[0] ?? (p.arm ? 'SRAM' : 'data (I/O + SRAM)')}</option>
        <option value="flash">{p.labels?.[1] ?? (p.arm ? 'Flash' : 'prog (Flash)')}</option>
        {p.extraRam?.map((r, i) => <option key={r.name} value={`ram${i + 1}`}>{r.name}</option>)}
        {p.eeprom && <option value="eeprom">eeprom (EEPROM)</option>}
        {!p.arm && <option value="nvm">fuses, lock, signature</option>}
      </select>
      {p.space !== 'nvm' && (
        <>
          <span>Columns:</span>
          <select className="w7-select" value={p.cols} onChange={(e) => p.setCols(Number(e.target.value))}>
            {[4, 8, 16, 32].map((c) => <option key={c} value={c}>{c}</option>)}
          </select>
          <span>Go to:</span>
          <input className="w7-input mono" style={{ width: 80 }} placeholder={p.placeholder ?? (p.arm ? '0x20000000' : '0x0040')} value={p.gotoText} onChange={(e) => p.setGotoText(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && p.onGoto()} />
          {p.note && <span className="dim">{p.note}</span>}
        </>
      )}
    </div>
  );
}
