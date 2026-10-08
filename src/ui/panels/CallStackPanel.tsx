import type { JSX } from 'react';
import { useSim } from '../state/sim';
import { requestDisasmGoto, requestGoto, useWorkspace } from '../state/workspace';
import { useLayout } from '../state/layout';
import { pcToSource, sameFile, baseName } from '../services/debugInfo';
import { hex } from '../format';
import { EmptyHint } from './common';

/** Shadow call stack tracked by the simulator (calls and interrupt entries). */
export function CallStackPanel(): JSX.Element {
  const st = useSim((s) => s.state);
  const spec = useSim((s) => s.spec);
  const build = useWorkspace((s) => s.build);
  if (!st) return <EmptyHint>No program running.</EmptyHint>;
  const frames = [...st.callStack].reverse();
  const describe = (wpc: number) => build?.symbols.describeCode(wpc * 2) ?? hex(wpc * 2, 4);
  const srcOf = (wpc: number) => {
    const s = build ? pcToSource(build.program, wpc) : null;
    return s ? `${baseName(s.file)}:${s.line}` : '';
  };
  const go = (wpc: number) => {
    const s = build ? pcToSource(build.program, wpc) : null;
    const doc = s && useWorkspace.getState().docs.find((d) => sameFile(s.file, d.path ?? d.name));
    if (s && doc) requestGoto(doc.id, s.line);
    else {
      useLayout.getState().show('disasm');
      requestDisasmGoto(wpc);
    }
  };
  const rows = [
    { name: describe(st.pc), pc: st.pc, note: 'current', src: srcOf(st.pc) },
    ...frames.map((f) => ({
      name: describe(f.returnPc),
      pc: f.returnPc,
      note: f.vector >= 0 ? `interrupted by ${spec?.vectors.find((v) => v.index === f.vector)?.name ?? `vector ${f.vector}`}` : `called ${describe(f.targetPc)}`,
      src: srcOf(f.returnPc),
    })),
  ];
  return (
    <div className="panel">
      <div className="panel-scroll">
        <table className="grid-table">
          <thead>
            <tr><th style={{ width: 18 }} /><th>Location</th><th>Address</th><th>Source</th><th>Note</th></tr>
          </thead>
          <tbody>
            {rows.map((r, i) => (
              <tr key={i} className="row-hot" onDoubleClick={() => go(r.pc)}>
                <td>{i === 0 && <span className="cm-exec-arrow inline" />}</td>
                <td>{r.name}</td>
                <td className="mono">{hex(r.pc * 2, 4)}</td>
                <td className="dim">{r.src}</td>
                <td className="dim">{r.note}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {frames.length === 0 && <div className="empty-hint">Call depth 0 (main program)</div>}
      </div>
    </div>
  );
}
