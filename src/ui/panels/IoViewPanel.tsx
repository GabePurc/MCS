import { useState, type JSX } from 'react';
import { useSim } from '../state/sim';
import { sim } from '../services/simClient';
import { bin8, hex } from '../format';
import { EditableValue, EmptyHint } from './common';
import type { IoRegisterSpec } from '../backend/types';
import { MmioIoView } from './MmioIoView';

const expandedGroups = new Set<string>(['PORTB', 'TC0']);
const expandedRegs = new Set<string>();
/** Device group -> peripheral model name (for its internal state). */
const PERIPH_FOR_GROUP: Record<string, string> = { CPU: 'SYSTEM' };

function popcount(m: number): number {
  let c = 0;
  for (; m; m &= m - 1) c++;
  return c;
}

/** Peripheral register view: the AVR (Atmel Studio style) or the memory-mapped variant (ARM, RISC-V). */
export function IoViewPanel(): JSX.Element {
  const arch = useSim((s) => s.spec?.arch);
  return arch === 'arm' || arch === 'riscv' ? <MmioIoView /> : <AvrIoView />;
}

/** Atmel Studio style I/O view: peripherals > registers > bits, live values, editable. */
function AvrIoView(): JSX.Element {
  const spec = useSim((s) => (s.spec?.arch === 'avr' ? s.spec : null));
  const st = useSim((s) => s.state);
  const base = useSim((s) => s.baseline);
  const [, force] = useState(0);
  const [filter, setFilter] = useState('');
  if (!spec || !st) return <EmptyHint>No device loaded.</EmptyHint>;
  const toggle = (set: Set<string>, k: string) => {
    if (set.has(k)) set.delete(k);
    else set.add(k);
    force((x) => x + 1);
  };
  const write = (r: IoRegisterSpec, v: number) => sim({ type: 'writeData', addr: r.addr, value: v & 0xff });
  const f = filter.trim().toUpperCase();
  return (
    <div className="panel">
      <div className="panel-toolbar">
        <input className="w7-input" style={{ flex: 1 }} placeholder="Filter registers / bits..." value={filter} onChange={(e) => setFilter(e.target.value)} />
      </div>
      <div className="panel-scroll">
        <table className="grid-table io-table">
          <thead>
            <tr>
              <th style={{ width: '45%' }}>Name</th>
              <th>Address</th>
              <th>Value</th>
              <th>Bits</th>
            </tr>
          </thead>
          <tbody>
            {spec.groups.map((g) => {
              const regs = spec.registers
                .filter((r) => r.group === g.name)
                .filter((r) => !f || r.name.includes(f) || r.bits.some((b) => b.name.includes(f)) || g.name.includes(f))
                .sort((a, b) => a.addr - b.addr);
              if (regs.length === 0) return null;
              const open = expandedGroups.has(g.name) || !!f;
              const info = st.peripherals.find((p) => p.name === (PERIPH_FOR_GROUP[g.name] ?? g.name));
              return [
                <tr key={g.name} className="io-group" onClick={() => toggle(expandedGroups, g.name)}>
                  <td colSpan={4}>
                    <span className={`w7-expander${open ? ' open' : ''}`} />
                    <b>{g.name}</b> <span className="dim">{g.desc}</span>
                  </td>
                </tr>,
                open && info && (
                  <tr key={`${g.name}-info`} className="io-info">
                    <td colSpan={4}>
                      {info.values.map(([k, v]) => (
                        <span key={k} className="io-chip">
                          <span className="dim">{k}:</span> {v}
                        </span>
                      ))}
                    </td>
                  </tr>
                ),
                ...(open
                  ? regs.flatMap((r) => {
                      const v = st.data[r.addr];
                      const was = base?.data[r.addr];
                      const ch = was !== undefined && was !== v;
                      const ropen = expandedRegs.has(r.name);
                      const io = r.addr - spec.ioBase;
                      const rows = [
                        <tr key={r.name} className="row-hot io-reg" data-tip={r.desc}>
                          <td style={{ paddingLeft: 18 }} onClick={() => r.bits.length && toggle(expandedRegs, r.name)}>
                            <span className={`w7-expander${ropen ? ' open' : ''}${r.bits.length ? '' : ' leaf'}`} />
                            {r.name}
                          </td>
                          <td className="mono dim">{io >= 0 && io < spec.ioSize ? hex(io) : ''} ({hex(r.addr, 4)})</td>
                          <td>
                            {r.access === 'w' ? (
                              <span className="dim">write-only</span>
                            ) : (
                              <EditableValue value={v} display={hex(v)} className={ch ? 'changed' : ''} onCommit={(nv) => write(r, nv)} title={`${r.name} = ${v} (0b${bin8(v)}). Double-click to write.`} />
                            )}
                          </td>
                          <td>
                            <span className="bit-boxes">
                              {Array.from({ length: 8 }, (_, i) => {
                                const bit = 7 - i;
                                const on = (v >> bit) & 1;
                                const bitCh = was !== undefined && ((was >> bit) & 1) !== on;
                                const named = r.bits.find((b) => b.mask & (1 << bit));
                                return (
                                  <span
                                    key={bit}
                                    className={`bit-box${on ? ' on' : ''}${bitCh ? ' changed' : ''}${named ? '' : ' reserved'}`}
                                    data-tip={`${named ? named.name : 'Reserved'} (bit ${bit})${named?.desc ? `\n${named.desc}` : ''}\nClick to toggle`}
                                    onClick={() => write(r, v ^ (1 << bit))}
                                  />
                                );
                              })}
                            </span>
                          </td>
                        </tr>,
                      ];
                      if (ropen) {
                        for (const b of r.bits) {
                          const shift = Math.log2(b.mask & -b.mask);
                          const val = (v & b.mask) >> shift;
                          const width = popcount(b.mask);
                          rows.push(
                            <tr key={`${r.name}.${b.name}`} className="io-bit row-hot" data-tip={b.desc}>
                              <td style={{ paddingLeft: 52 }}>{b.name}</td>
                              <td className="mono dim">{width === 1 ? `bit ${shift}` : `bits ${shift + width - 1}:${shift}`}</td>
                              <td className="mono">{width === 1 ? val : `${val} (0b${val.toString(2).padStart(width, '0')})`}</td>
                              <td className="dim ellipsis">{b.desc}</td>
                            </tr>,
                          );
                        }
                      }
                      return rows;
                    })
                  : []),
              ];
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
