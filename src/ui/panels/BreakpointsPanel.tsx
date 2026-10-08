import type { JSX } from 'react';
import { clearBreakpoints, removeBreakpoint, requestDisasmGoto, requestGoto, resolveBreakpoint, setBreakpointEnabled, useWorkspace } from '../state/workspace';
import { useLayout } from '../state/layout';
import { baseName, sameFile } from '../services/debugInfo';
import { hex } from '../format';
import { Icons } from '../icons';
import { EmptyHint } from './common';

export function BreakpointsPanel(): JSX.Element {
  const bps = useWorkspace((s) => s.breakpoints);
  const build = useWorkspace((s) => s.build);
  return (
    <div className="panel">
      <div className="panel-toolbar">
        <button className="tb-btn" data-tip="Delete all breakpoints" onClick={clearBreakpoints} disabled={!bps.length}>
          <Icons.ClearBreakpoints /> Delete all
        </button>
        <button className="tb-btn" data-tip="Disable all breakpoints" disabled={!bps.length} onClick={() => bps.forEach((b) => setBreakpointEnabled(b.id, false))}>
          Disable all
        </button>
        <button className="tb-btn" data-tip="Enable all breakpoints" disabled={!bps.length} onClick={() => bps.forEach((b) => setBreakpointEnabled(b.id, true))}>
          Enable all
        </button>
      </div>
      {bps.length === 0 ? (
        <EmptyHint>No breakpoints. Click the margin left of a source line or press F9.</EmptyHint>
      ) : (
        <div className="panel-scroll">
          <table className="grid-table">
            <thead>
              <tr><th style={{ width: 22 }} /><th>Location</th><th>Address</th><th style={{ width: 22 }} /></tr>
            </thead>
            <tbody>
              {bps.map((b) => {
                const pc = resolveBreakpoint(b, build?.program ?? null);
                return (
                  <tr
                    key={b.id}
                    className="row-hot"
                    onDoubleClick={() => {
                      if (b.kind === 'source') {
                        const doc = useWorkspace.getState().docs.find((d) => sameFile(b.file, d.path ?? d.name));
                        if (doc) requestGoto(doc.id, b.line);
                      } else {
                        useLayout.getState().show('disasm');
                        requestDisasmGoto(b.pc);
                      }
                    }}
                  >
                    <td><label className="w7-check"><input type="checkbox" checked={b.enabled} onChange={(e) => setBreakpointEnabled(b.id, e.target.checked)} /></label></td>
                    <td>{b.kind === 'source' ? `${baseName(b.file)}, line ${b.line}` : `Address ${hex(b.pc * 2, 4)}`}</td>
                    <td className="mono">{pc >= 0 ? hex(pc * 2, 4) : <span className="dim">unresolved</span>}</td>
                    <td>
                      <button className="tb-btn" data-tip="Delete" onClick={() => removeBreakpoint(b.id)}><Icons.Close size={8} /></button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
