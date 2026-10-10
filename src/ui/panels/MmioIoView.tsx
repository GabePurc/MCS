import { useMemo, useState, type JSX } from 'react';
import { useSim } from '../state/sim';
import { sim } from '../services/simClient';
import { hex } from '../format';
import { EditableValue, EmptyHint } from './common';
import type { ArmDeviceSpec, MmioRegisterSpec, RiscvDeviceSpec } from '../backend/types';

const expandedGroups = new Set<string>(['GPIOA', 'GPIO']);
const expandedRegs = new Set<string>();

function popcount(m: number): number {
  let c = 0;
  for (; m; m &= m - 1) c++;
  return c;
}

/** `v` with the bits of `mask` replaced by `field`, as unsigned 32-bit. */
function setField(v: number, mask: number, field: number): number {
  const shift = 31 - Math.clz32(mask & -mask);
  return ((v & ~mask) | ((field << shift) & mask)) >>> 0;
}

interface Entry {
  reg: MmioRegisterSpec;
  /** Index into `spec.registers` / `state.io`. */
  idx: number;
}

/** Peripheral register view for memory-mapped devices (ARM, RISC-V): group > register (32-bit values from `state.io`) > bit fields. */
export function MmioIoView(): JSX.Element {
  const spec = useSim((s) => (s.spec && s.spec.arch !== 'avr' ? s.spec : null));
  const io = useSim((s) => s.state?.io);
  const peripherals = useSim((s) => s.state?.peripherals);
  const baseIo = useSim((s) => s.baseline?.io);
  const [, force] = useState(0);
  const [filter, setFilter] = useState('');
  const byGroup = useMemo(() => (spec ? groupRegisters(spec) : new Map<string, Entry[]>()), [spec]);
  if (!spec || !io || io.length === 0) return <EmptyHint>No device loaded.</EmptyHint>;
  const toggle = (set: Set<string>, k: string) => {
    if (set.has(k)) set.delete(k);
    else set.add(k);
    force((x) => x + 1);
  };
  const write = (r: MmioRegisterSpec, v: number) => sim({ type: 'writeMem', addr: r.addr, size: r.size, value: v >>> 0 });
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
              <th style={{ width: '38%' }}>Name</th>
              <th>Address</th>
              <th>Value</th>
              <th>Bits</th>
            </tr>
          </thead>
          <tbody>
            {spec.groups.map((g) => {
              const all = byGroup.get(g.name);
              if (!all) return null;
              const regs = f ? all.filter(({ reg }) => reg.name.includes(f) || g.name.includes(f) || reg.bits.some((b) => b.name.includes(f))) : all;
              if (regs.length === 0) return null;
              const open = expandedGroups.has(g.name) || !!f;
              const info = open ? peripherals?.find((p) => p.name === g.name) : undefined;
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
                ...(open ? regs.flatMap(({ reg: r, idx }) => registerRows(r, io[idx] ?? 0, baseIo?.[idx], write, expandedRegs.has(r.name), () => toggle(expandedRegs, r.name))) : []),
              ];
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function registerRows(r: MmioRegisterSpec, v: number, was: number | undefined, write: (r: MmioRegisterSpec, v: number) => void, ropen: boolean, toggleReg: () => void): JSX.Element[] {
  const bits = r.size * 8;
  const digits = r.size * 2;
  const ch = was !== undefined && was !== v;
  const rows: JSX.Element[] = [
    <tr key={r.name} className="row-hot io-reg" data-tip={r.desc}>
      <td style={{ paddingLeft: 18 }} onClick={() => r.bits.length && toggleReg()}>
        <span className={`w7-expander${ropen ? ' open' : ''}${r.bits.length ? '' : ' leaf'}`} />
        {r.name}
      </td>
      <td className="mono dim">{hex(r.addr, 8)}</td>
      <td>
        {r.access === 'w' ? (
          <span className="dim">write-only</span>
        ) : (
          <EditableValue value={v} display={hex(v, digits)} max={r.size === 4 ? 0xffffffff : (1 << bits) - 1} className={ch ? 'changed' : ''} onCommit={(nv) => write(r, nv)} title={`${r.name} = ${v} (${hex(v, digits)}). Double-click to write.`} />
        )}
      </td>
      <td>
        <span className="bit-boxes wide">
          {Array.from({ length: bits }, (_, i) => {
            const bit = bits - 1 - i;
            const on = (v >>> bit) & 1;
            const bitCh = was !== undefined && ((was >>> bit) & 1) !== on;
            const named = r.bits.find((b) => (b.mask >>> bit) & 1);
            return (
              <span
                key={bit}
                className={`bit-box${on ? ' on' : ''}${bitCh ? ' changed' : ''}${named ? '' : ' reserved'}`}
                data-tip={`${named ? named.name : 'Reserved'} (bit ${bit})${named?.desc ? `\n${named.desc}` : ''}\nClick to toggle`}
                onClick={() => write(r, (v ^ (1 << bit)) >>> 0)}
              />
            );
          })}
        </span>
      </td>
    </tr>,
  ];
  if (ropen) {
    for (const b of r.bits) {
      const shift = 31 - Math.clz32(b.mask & -b.mask);
      const width = popcount(b.mask);
      const val = ((v & b.mask) >>> shift) >>> 0;
      rows.push(
        <tr key={`${r.name}.${b.name}`} className="io-bit row-hot" data-tip={b.desc}>
          <td style={{ paddingLeft: 52 }}>{b.name}</td>
          <td className="mono dim">{width === 1 ? `bit ${shift}` : `bits ${shift + width - 1}:${shift}`}</td>
          <td>
            {r.access === 'w' ? null : (
              <EditableValue value={val} display={width === 1 ? String(val) : `${val} (0b${val.toString(2).padStart(width, '0')})`} max={width >= 32 ? 0xffffffff : 2 ** width - 1} onCommit={(nv) => write(r, setField(v, b.mask, nv))} title={`${b.name} = ${val}. Double-click to write.`} />
            )}
          </td>
          <td className="dim ellipsis">{b.desc}</td>
        </tr>,
      );
    }
  }
  return rows;
}

function groupRegisters(spec: ArmDeviceSpec | RiscvDeviceSpec): Map<string, Entry[]> {
  const m = new Map<string, Entry[]>();
  spec.registers.forEach((reg, idx) => {
    const a = m.get(reg.group);
    if (a) a.push({ reg, idx });
    else m.set(reg.group, [{ reg, idx }]);
  });
  for (const a of m.values()) a.sort((x, y) => x.reg.addr - y.reg.addr);
  return m;
}
