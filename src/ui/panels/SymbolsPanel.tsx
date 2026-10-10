import { useState, type JSX } from 'react';
import { useSim } from '../state/sim';
import { requestDisasmGoto, toggleAddressBreakpoint, useWorkspace } from '../state/workspace';
import { useLayout } from '../state/layout';
import { hex } from '../format';
import { bytesToPc } from '../backend/types';
import { EmptyHint, Section } from './common';
import { openContextMenu } from '../controls/Menu';

/** Program symbols: data objects with live values (a watch list) and code labels/functions. */
export function SymbolsPanel(): JSX.Element {
  const build = useWorkspace((s) => s.build);
  const st = useSim((s) => s.state);
  const base = useSim((s) => s.baseline);
  const spec = useSim((s) => s.spec);
  const arch = spec?.arch ?? 'avr';
  const dataBase = spec?.arch === 'arm' ? spec.sramBase : 0;
  const addrDigits = arch === 'arm' ? 8 : 4;
  const [filter, setFilter] = useState('');
  if (!build) return <EmptyHint>Build a program to list its symbols.</EmptyHint>;
  const f = filter.trim().toLowerCase();
  const match = (n: string) => !f || n.toLowerCase().includes(f);
  const data = build.symbols.data.filter((s) => match(s.name));
  const code = build.symbols.code.filter((s) => match(s.name));
  const consts = build.program.symbols.filter((s) => s.kind === 'const' && match(s.name));
  const valueOf = (addr: number, size: number, src: Uint8Array | undefined) => {
    if (!src) return null;
    let v = 0;
    const n = Math.min(Math.max(size, 1), 4);
    for (let i = n - 1; i >= 0; i--) v = v * 256 + (src[addr - dataBase + i] ?? 0);
    return { v, n };
  };
  return (
    <div className="panel">
      <div className="panel-toolbar">
        <input className="w7-input" style={{ flex: 1 }} placeholder="Filter symbols..." value={filter} onChange={(e) => setFilter(e.target.value)} />
      </div>
      <div className="panel-scroll">
        <Section title={`Variables (${data.length})`}>
          <table className="grid-table">
            <thead>
              <tr><th>Name</th><th>Address</th><th>Size</th><th>Value</th></tr>
            </thead>
            <tbody>
              {data.map((s) => {
                const cur = valueOf(s.address, s.size, st?.data);
                const old = valueOf(s.address, s.size, base?.data);
                return (
                  <tr key={`${s.name}@${s.address}`} className="row-hot" onDoubleClick={() => useLayout.getState().show('memory')}>
                    <td>{s.name}</td>
                    <td className="mono">{hex(s.address, addrDigits)}</td>
                    <td>{s.size || '?'}</td>
                    <td className={`mono${cur && old && cur.v !== old.v ? ' changed' : ''}`}>{cur ? `${hex(cur.v, cur.n * 2)} (${cur.v})` : '-'}</td>
                  </tr>
                );
              })}
              {data.length === 0 && <tr><td colSpan={4} className="dim">No data symbols</td></tr>}
            </tbody>
          </table>
        </Section>
        <Section title={`Code (${code.length})`}>
          <table className="grid-table">
            <thead>
              <tr><th>Name</th><th>Address</th><th>Kind</th></tr>
            </thead>
            <tbody>
              {code.map((s) => (
                <tr
                  key={`${s.name}@${s.address}`}
                  className="row-hot"
                  onDoubleClick={() => {
                    useLayout.getState().show('disasm');
                    requestDisasmGoto(bytesToPc(arch, s.address));
                  }}
                  onContextMenu={(e) => openContextMenu(e, [{ kind: 'action', label: 'Toggle Breakpoint', icon: 'Breakpoint', run: () => toggleAddressBreakpoint(bytesToPc(arch, s.address)) }])}
                >
                  <td>{s.name}</td>
                  <td className="mono">{hex(s.address, addrDigits)}</td>
                  <td className="dim">{s.kind}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Section>
        {consts.length > 0 && (
          <Section title={`Constants (${consts.length})`}>
            <table className="grid-table">
              <tbody>
                {consts.map((s) => (
                  <tr key={s.name} className="row-hot"><td>{s.name}</td><td className="mono">{hex(s.address, 2)} ({s.address})</td></tr>
                ))}
              </tbody>
            </table>
          </Section>
        )}
      </div>
    </div>
  );
}
