import { useEffect, useMemo, useRef, useState, type JSX } from 'react';
import { disassemble } from '../backend/api';
import { flashBaseOf, pcToBytes, type DisasmLine } from '../backend/types';
import { useSim } from '../state/sim';
import { appendOutput, requestGoto, resolvedPcMap, toggleAddressBreakpoint, useWorkspace } from '../state/workspace';
import { sim } from '../services/simClient';
import { baseName, pcToSource, sameFile } from '../services/debugInfo';
import { openPath } from '../services/files';
import { hexRaw } from '../format';
import { openContextMenu } from '../controls/Menu';
import { EmptyHint } from './common';

const ROW_H = 18;

type Row = { kind: 'label'; text: string } | { kind: 'src'; text: string } | { kind: 'insn'; line: DisasmLine };

/** Live disassembly of program memory with labels, breakpoints and the current PC. */
export function DisassemblyPanel(): JSX.Element {
  const spec = useSim((s) => s.spec);
  const flash = useSim((s) => s.flash);
  const pc = useSim((s) => s.state?.pc ?? 0);
  const running = useSim((s) => s.running);
  const revealSeq = useSim((s) => s.revealSeq);
  const build = useWorkspace((s) => s.build);
  const arch = spec?.arch ?? 'avr';
  const flashBase = spec ? flashBaseOf(spec) : 0;
  const addrDigits = arch === 'arm' ? 8 : 4;
  const bps = useWorkspace((s) => s.breakpoints);
  const disasmGoto = useWorkspace((s) => s.disasmGoto);
  const [lines, setLines] = useState<DisasmLine[]>([]);
  const [scroll, setScroll] = useState(0);
  const [height, setHeight] = useState(300);
  const [showAll, setShowAll] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);

  // Re-disassemble when the image or labels change (rare).
  useEffect(() => {
    if (!spec || !flash) return;
    const labels: Record<number, string> = {};
    for (const s of build?.symbols.code ?? []) if (!(s.address in labels)) labels[s.address] = s.name;
    let cancelled = false;
    disassemble(spec.id, flash, labels)
      .then((l) => !cancelled && setLines(l))
      .catch((e) => appendOutput('error', `Disassembly failed: ${e instanceof Error ? e.message : String(e)}`));
    return () => {
      cancelled = true;
    };
  }, [spec, flash, build]);

  const used = build?.program.flashUsed ?? 0;
  const rows = useMemo<Row[]>(() => {
    const out: Row[] = [];
    const labelAt = new Map<number, string[]>();
    for (const s of build?.symbols.code ?? []) {
      const arr = labelAt.get(s.address) ?? [];
      if (!arr.includes(s.name)) arr.push(s.name);
      labelAt.set(s.address, arr);
    }
    let lastSrc = '';
    for (const l of lines) {
      const byte = pcToBytes(arch, l.pc);
      if (!showAll && byte - flashBase >= used && l.raw[0] === 0xffff) continue;
      for (const n of labelAt.get(byte) ?? []) out.push({ kind: 'label', text: `${n}:` });
      const src = build ? pcToSource(build.program, l.pc, arch) : null;
      if (src) {
        const key = `${src.file}:${src.line}`;
        if (key !== lastSrc) {
          out.push({ kind: 'src', text: `${baseName(src.file)}:${src.line}` });
          lastSrc = key;
        }
      }
      out.push({ kind: 'insn', line: l });
    }
    return out;
  }, [lines, build, used, showAll, arch, flashBase]);

  const rowOfPc = useMemo(() => {
    const m = new Map<number, number>();
    rows.forEach((r, i) => r.kind === 'insn' && m.set(r.line.pc, i));
    return m;
  }, [rows]);

  const bpMap = useMemo(() => resolvedPcMap(bps, build?.program ?? null, build?.arch), [bps, build]);

  // Scroll the current PC into view whenever execution stops.
  useEffect(() => {
    const el = scroller.current;
    const i = rowOfPc.get(pc);
    if (!el || i === undefined || running) return;
    const y = i * ROW_H;
    if (y < el.scrollTop || y > el.scrollTop + el.clientHeight - ROW_H * 2) el.scrollTop = Math.max(0, y - el.clientHeight / 3);
  }, [revealSeq, rowOfPc]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const el = scroller.current;
    const i = disasmGoto ? rowOfPc.get(disasmGoto.pc) : undefined;
    if (el && i !== undefined) el.scrollTop = Math.max(0, i * ROW_H - el.clientHeight / 3);
  }, [disasmGoto]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!spec || !flash) return <EmptyHint>No program loaded.</EmptyHint>;

  const first = Math.max(0, Math.floor(scroll / ROW_H) - 2);
  const last = Math.min(rows.length, first + Math.ceil(height / ROW_H) + 4);

  const gotoSource = (wpc: number) => {
    const loc = build ? pcToSource(build.program, wpc, arch) : null;
    if (!loc) return;
    const ws = useWorkspace.getState();
    const doc = ws.docs.find((d) => sameFile(loc.file, d.path ?? d.name));
    if (doc) requestGoto(doc.id, loc.line);
    else void openPath(loc.file).then((ok) => ok && requestGoto(useWorkspace.getState().activeDocId!, loc.line));
  };

  return (
    <div className="panel">
      <div className="panel-toolbar">
        <label className="w7-check">
          <input type="checkbox" checked={showAll} onChange={(e) => setShowAll(e.target.checked)} />
          Show unprogrammed flash
        </label>
        <span className="dim" style={{ marginLeft: 'auto' }}>{used} / {spec.flashSize} bytes used</span>
      </div>
      <div
        className="panel-scroll disasm mono"
        ref={(el) => {
          scroller.current = el;
          if (el && el.clientHeight !== height) setHeight(el.clientHeight);
        }}
        onScroll={(e) => setScroll(e.currentTarget.scrollTop)}
      >
        <div style={{ height: rows.length * ROW_H, position: 'relative' }}>
          {rows.slice(first, last).map((r, k) => {
            const top = (first + k) * ROW_H;
            if (r.kind === 'label') return <div key={`l${first + k}`} className="dis-row dis-label" style={{ top }}>{r.text}</div>;
            if (r.kind === 'src') return <div key={`s${first + k}`} className="dis-row dis-src" style={{ top }}>{r.text}</div>;
            const l = r.line;
            const isPc = l.pc === pc;
            const bp = bpMap.has(l.pc);
            return (
              <div
                key={`i${l.pc}`}
                className={`dis-row dis-insn${isPc && !running ? ' current' : ''}${l.valid ? '' : ' invalid'}`}
                style={{ top }}
                onDoubleClick={() => gotoSource(l.pc)}
                onContextMenu={(e) =>
                  openContextMenu(e, [
                    { kind: 'action', label: 'Toggle Breakpoint', icon: 'Breakpoint', run: () => toggleAddressBreakpoint(l.pc) },
                    { kind: 'action', label: 'Run To Here', icon: 'RunToCursor', disabled: running, run: () => sim({ type: 'runTo', pc: l.pc }) },
                    { kind: 'action', label: 'Set Next Statement (PC)', disabled: running, run: () => sim({ type: 'writeCpu', field: 'pc', value: pcToBytes(arch, l.pc) }) },
                    { kind: 'sep' },
                    { kind: 'action', label: 'Go To Source', run: () => gotoSource(l.pc) },
                  ])
                }
              >
                <span className="dis-gutter" onMouseDown={() => toggleAddressBreakpoint(l.pc)}>
                  {bp && <span className="cm-bp" />}
                  {isPc && !running && <span className="cm-exec-arrow" />}
                </span>
                <span className="dis-addr">{hexRaw(pcToBytes(arch, l.pc), addrDigits)}</span>
                <span className="dis-raw dim">{l.raw.map((w) => hexRaw(w, 4)).join(' ')}</span>
                <span className="dis-mn">{l.mnemonic}</span>
                <span className="dis-ops">{l.operands}</span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
