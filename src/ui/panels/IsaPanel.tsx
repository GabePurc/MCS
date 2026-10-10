import { useEffect, useMemo, useState, type JSX } from 'react';
import { buildAsm, instructionSet, machineCodeHints } from '../backend/api';
import type { InsnInfo } from '../backend/types';
import { useSim } from '../state/sim';
import { activeDoc } from '../state/workspace';
import { editorApi } from '../editor/editorApi';
import { hexRaw } from '../format';
import { EmptyHint, Section } from './common';

/** Colours the operand fields of an encoding pattern ("1110 KKKK dddd KKKK"). */
function Encoding({ pattern }: { pattern: string }): JSX.Element {
  return (
    <span className="mono enc">
      {[...pattern].map((c, i) => (c === ' ' ? ' ' : <span key={i} className={/[01]/.test(c) ? 'enc-fixed' : `enc-field enc-${c.toLowerCase()}`}>{c}</span>))}
    </span>
  );
}

const bin16 = (w: number) => w.toString(2).padStart(16, '0').replace(/(.{4})(?!$)/g, '$1 ');

/** Assembles one instruction to machine code (and back), for writing `.mc` files by hand. */
function Converter({ deviceId }: { deviceId: string }): JSX.Element {
  const [asm, setAsm] = useState('ldi r16, 0x0F');
  const [mc, setMc] = useState('E00F');
  const [enc, setEnc] = useState<{ words: number[]; error?: string }>({ words: [] });
  const [dec, setDec] = useState('');
  useEffect(() => {
    const t = setTimeout(() => {
      if (!asm.trim()) return setEnc({ words: [] });
      buildAsm(`${asm}\n`, 'encode.asm', null, deviceId)
        .then((r) => {
          const err = r.diagnostics.find((d) => d.severity === 'error');
          if (err || !r.program) return setEnc({ words: [], error: err?.message ?? 'cannot encode' });
          const f = r.program.flash;
          const n = Math.ceil(r.program.flashUsed / 2);
          setEnc({ words: Array.from({ length: n }, (_, i) => f[i * 2] | (f[i * 2 + 1] << 8)) });
        })
        .catch((e) => setEnc({ words: [], error: String(e) }));
    }, 150);
    return () => clearTimeout(t);
  }, [asm, deviceId]);
  useEffect(() => {
    const t = setTimeout(() => {
      if (!mc.trim()) return setDec('');
      machineCodeHints(mc, deviceId)
        .then((r) => setDec(r.diagnostics.find((d) => d.severity === 'error')?.message ?? r.hints.map((h) => h.text).join('  |  ')))
        .catch((e) => setDec(String(e)));
    }, 150);
    return () => clearTimeout(t);
  }, [mc, deviceId]);
  return (
    <div className="isa-converter">
      <span>Assembly:</span>
      <input className="w7-input mono" value={asm} onChange={(e) => setAsm(e.target.value)} placeholder="ldi r16, 0x0F" />
      <span className="mono isa-out selectable" data-tip="Machine code words (hex and binary)">
        {enc.error ? <span className="error-text">{enc.error}</span> : enc.words.map((w) => `${hexRaw(w, 4)}  (${bin16(w)})`).join('   ')}
      </span>
      <span>Machine code:</span>
      <input className="w7-input mono" value={mc} onChange={(e) => setMc(e.target.value)} placeholder="E00F or 0b1110..." />
      <span className="mono isa-out selectable">{dec}</span>
    </div>
  );
}

/** Instruction set reference (non-modal, dockable/floating) with encoder/decoder. */
export function IsaPanel(): JSX.Element {
  const spec = useSim((s) => s.spec);
  const [rows, setRows] = useState<InsnInfo[]>([]);
  const [filter, setFilter] = useState('');
  const [sel, setSel] = useState<string | null>(null);
  const [showAliases, setShowAliases] = useState(true);
  useEffect(() => {
    if (spec) instructionSet(spec.id).then(setRows).catch(() => {});
  }, [spec]);
  const shown = useMemo(() => {
    const f = filter.trim().toLowerCase();
    const base = showAliases ? rows : rows.filter((r) => !r.aliasOf);
    if (!f) return base;
    return base.filter((r) => r.mnemonic.toLowerCase().includes(f) || r.summary.toLowerCase().includes(f) || r.aliases.toLowerCase().includes(f));
  }, [rows, filter, showAliases]);
  const current = rows.find((r) => `${r.mnemonic} ${r.operands}` === sel);
  if (!spec) return <EmptyHint>No device loaded.</EmptyHint>;
  const insert = (r: InsnInfo) => {
    const doc = activeDoc();
    const api = editorApi();
    if (api && doc?.language === 'asm') api.insertText(`${r.mnemonic.toLowerCase()} `);
    else void navigator.clipboard?.writeText(r.mnemonic.toLowerCase());
  };
  return (
    <div className="panel">
      <div className="panel-toolbar">
        <input className="w7-input" placeholder="Filter (mnemonic, description, alias)..." value={filter} onChange={(e) => setFilter(e.target.value)} style={{ flex: 1 }} />
        <label className="dim" style={{ whiteSpace: 'nowrap' }} data-tip="Also list assembler aliases such as BRNE, CLR and SEI"><input type="checkbox" checked={showAliases} onChange={(e) => setShowAliases(e.target.checked)} /> Aliases</label>
        <span className="dim" style={{ whiteSpace: 'nowrap' }}>{rows.filter((r) => !r.aliasOf).length} instructions</span>
      </div>
      <Converter deviceId={spec.id} />
      <div className="panel-scroll">
        <table className="grid-table isa-table">
          <thead>
            <tr><th>Mnemonic</th><th>Operands</th><th>Description</th><th>Encoding</th><th>Cyc</th><th>Words</th></tr>
          </thead>
          <tbody>
            {shown.map((r) => {
              const key = `${r.mnemonic} ${r.operands}`;
              return (
                <tr key={key} className={`row-hot${sel === key ? ' selected' : ''}`} onClick={() => setSel(key)} onDoubleClick={() => insert(r)} data-tip={`${r.operation}${r.aliases ? `\nAliases: ${r.aliases}` : ''}${r.aliasOf ? `\nAlias of ${r.aliasOf}` : ''}\nDouble-click to insert into the editor`}>
                  <td>{r.aliasOf ? <i>{r.mnemonic}</i> : <b>{r.mnemonic}</b>}</td>
                  <td className="mono">{r.operands}</td>
                  <td>{r.summary}</td>
                  <td><Encoding pattern={r.encoding} /></td>
                  <td>{r.cycles}</td>
                  <td>{r.words}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {current && (
        <div className="isa-detail">
          <Section title={`${current.mnemonic} ${current.operands} - ${current.summary}`} />
          <div className="form-grid">
            <span>Operation:</span><span className="mono selectable">{current.operation}</span>
            <span>Flags:</span><span className="mono">{current.flags}</span>
            <span>Encoding:</span><Encoding pattern={current.encoding} />
            {current.aliases && (<><span>Aliases:</span><span className="mono">{current.aliases}</span></>)}
            {current.aliasOf && (<><span>Alias of:</span><span className="mono">{current.aliasOf}</span></>)}
            {current.usage && (<><span>How to use:</span><span>{current.usage}</span></>)}
            {current.example && (<><span>Example:</span><pre className="mono selectable isa-example">{current.example}</pre></>)}
          </div>
          <p className="dim" style={{ margin: '4px 0 0' }}>Letters in the encoding are operand bits: d = destination register, r = source register, K = constant, k = address, A = I/O address, b = bit, s = SREG bit, q = displacement. Branches take one extra cycle when taken; skips take 1 + the skipped instruction's size.</p>
        </div>
      )}
    </div>
  );
}
