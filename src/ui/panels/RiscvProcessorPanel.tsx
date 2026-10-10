import type { JSX } from 'react';
import { riscvCore, type CpuField } from '../backend/types';
import { useSim, resetStopwatch } from '../state/sim';
import { sim } from '../services/simClient';
import { useWorkspace } from '../state/workspace';
import { formatHz, formatTime, hex } from '../format';
import { EditableValue, EmptyHint, Flag, Section } from './common';
import { Icons } from '../icons';
import { decodeMcause, interruptLines, MPP_NAMES, MSTATUS_FLAGS, mstatusMpp, mtvecInfo, rangeList, riscvRegName, RISCV_ROLE } from '../services/riscvState';

const U32 = 0xffffffff;
const changed = (a: number, b: number | undefined) => (b !== undefined && a !== b ? 'changed' : '');

function write(field: CpuField, value: number): void {
  sim({ type: 'writeCpu', field, value: value >>> 0 });
}

/** Bit boxes for an interrupt mask (bit 0 is not an interrupt line); `onToggle` makes them clickable. */
function MaskBits({ value, was, name, onToggle }: { value: number; was?: number; name: string; onToggle?: (bit: number) => void }): JSX.Element {
  return (
    <span className="bit-boxes wide">
      {Array.from({ length: 32 }, (_, i) => {
        const bit = 31 - i;
        const on = (value >>> bit) & 1;
        const ch = was !== undefined && ((was >>> bit) & 1) !== on;
        return (
          <span
            key={bit}
            className={`bit-box${on ? ' on' : ''}${ch ? ' changed' : ''}${bit === 0 ? ' reserved' : ''}`}
            data-tip={bit === 0 ? `${name} bit 0 (not an interrupt line)` : `${name}: CPU interrupt ${bit}${onToggle ? '\nClick to toggle' : ''}`}
            onClick={onToggle && bit !== 0 ? () => onToggle(bit) : undefined}
          />
        );
      })}
    </span>
  );
}

/** RV32IMC processor view: x0-x31 with ABI names, pc, machine-mode CSRs. */
export function RiscvProcessorPanel(): JSX.Element {
  const st = useSim((s) => s.state);
  const base = useSim((s) => s.baseline);
  const spec = useSim((s) => s.spec);
  const stopwatch = useSim((s) => s.stopwatch);
  const symbols = useWorkspace((s) => s.build?.symbols);
  if (!st || !spec || spec.arch !== 'riscv' || st.core.arch !== 'riscv') return <EmptyHint>Load a program to see the processor state.</EmptyHint>;
  const c = riscvCore(st);
  const b = base && base.core.arch === 'riscv' ? base.core : null;
  const swCycles = st.cycles - stopwatch.cycles;
  const swTime = st.timeSec - stopwatch.time;
  const cause = decodeMcause(c.mcause);
  const tv = mtvecInfo(c.mtvec);
  const mpp = mstatusMpp(c.mstatus);
  const enabled = interruptLines(c.mie);
  const pending = interruptLines(c.mip);
  const rows: [string, JSX.Element | string, string?][] = [
    [
      'Program Counter',
      <EditableValue value={st.pc} display={hex(st.pc, 8)} max={U32} title="Byte address (double-click to edit)" className={changed(st.pc, base?.pc)} onCommit={(v) => write('pc', v & ~1)} />,
      symbols?.describeCode(st.pc),
    ],
    ['Cycle Counter', <span className="mono">{st.cycles.toLocaleString()}</span>, 'mcycle'],
    ['Instructions', <span className="mono">{st.instructions.toLocaleString()}</span>, 'minstret'],
    ['Frequency', formatHz(st.hz)],
    [
      'Stop Watch',
      <span className="mono">
        {formatTime(swTime)} ({swCycles.toLocaleString()} cyc)
      </span>,
    ],
    ['Elapsed', <span className="mono">{formatTime(st.timeSec)}</span>],
    ['State', st.sleeping ? 'Sleeping (wfi)' : st.running ? 'Running' : 'Stopped'],
  ];
  return (
    <div className="panel">
      <div className="panel-scroll">
        <Section
          title="Processor"
          right={
            <button className="tb-btn" data-tip="Reset stop watch" onClick={resetStopwatch}>
              <Icons.Reset size={14} />
            </button>
          }
        >
          <table className="grid-table kv">
            <tbody>
              {rows.map(([k, v, extra]) => (
                <tr key={k} className="row-hot">
                  <td className="dim">{k}</td>
                  <td>
                    {v}
                    {extra && <span className="dim"> {extra}</span>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Section>
        <Section title="Registers">
          <div className="reg-grid fp">
            {Array.from({ length: 32 }, (_, i) => {
              const v = c.x[i];
              const tip = `x${i} (${riscvRegName(i)}): ${RISCV_ROLE[i]}\n${v} (${v | 0} signed)`;
              return (
                <div key={i} className="reg-cell">
                  <span className="reg-name">{riscvRegName(i)}</span>
                  {i === 0 ? (
                    <span className="mono dim" data-tip={tip}>{hex(0, 8)}</span>
                  ) : (
                    <EditableValue
                      value={v}
                      display={hex(v, 8)}
                      max={U32}
                      title={`${tip}${i === 1 && v ? `\n${symbols?.describeCode(v & ~1) ?? ''}` : ''}\n(double-click to edit)`}
                      className={changed(v, b?.x[i])}
                      onCommit={(nv) => sim({ type: 'writeReg', reg: i, value: nv >>> 0 })}
                    />
                  )}
                  <span className="reg-extra mono">x{i}</span>
                </div>
              );
            })}
          </div>
        </Section>
        <Section title="Machine Status (mstatus)">
          <div className="psr-row">
            {MSTATUS_FLAGS.map(([n, bit, tip]) => (
              <Flag key={n} name={n} on={((c.mstatus >>> bit) & 1) === 1} was={b ? ((b.mstatus >>> bit) & 1) === 1 : undefined} tip={tip} onClick={() => write('mstatus', c.mstatus ^ (1 << bit))} />
            ))}
            <span className="dim">MPP</span>
            <b className="mono" data-tip="Previous privilege mode (the core is machine-mode only)">{mpp}</b>
            <span className="dim">{MPP_NAMES[mpp]}</span>
            <span className="mono sreg-hex">{hex(c.mstatus, 8)}</span>
          </div>
        </Section>
        <Section title="Interrupts">
          <div className="psr-info">
            <div style={{ marginBottom: 4 }}>
              <span className="dim">mie </span>
              <EditableValue value={c.mie} display={hex(c.mie, 8)} max={U32} className={changed(c.mie, b?.mie)} onCommit={(v) => write('mie', v)} title="Machine interrupt enable (one bit per CPU interrupt line; double-click to edit)" />
              <span className="dim"> enabled: {enabled.length === 0 ? 'none' : enabled.length === 31 ? 'all (1-31)' : rangeList(enabled)}</span>
            </div>
            <MaskBits value={c.mie} was={b?.mie} name="mie" onToggle={(bit) => write('mie', c.mie ^ (1 << bit))} />
            <div style={{ margin: '6px 0 4px' }}>
              <span className="dim">mip </span>
              <span className={`mono ${changed(c.mip, b?.mip)}`} data-tip="Machine interrupt pending (driven by the interrupt controller, read-only)">{hex(c.mip, 8)}</span>
              <span className="dim"> pending: {pending.length === 0 ? 'none' : rangeList(pending)}</span>
            </div>
            <MaskBits value={c.mip} was={b?.mip} name="mip" />
          </div>
        </Section>
        <Section title="Trap CSRs">
          <table className="grid-table kv">
            <tbody>
              <tr className="row-hot">
                <td className="dim">mtvec</td>
                <td>
                  <EditableValue value={c.mtvec} display={hex(c.mtvec, 8)} max={U32} className={changed(c.mtvec, b?.mtvec)} onCommit={(v) => write('mtvec', v)} title="Trap vector: BASE in bits 31:2, MODE in bits 1:0 (double-click to edit)" />
                  <span className="dim"> {tv.mode}, base {hex(tv.base, 8)}{tv.mode === 'vectored' ? ' (interrupts at base + 4 * cause)' : ''}</span>
                </td>
              </tr>
              <tr className="row-hot">
                <td className="dim">mepc</td>
                <td>
                  <EditableValue value={c.mepc} display={hex(c.mepc, 8)} max={U32} className={changed(c.mepc, b?.mepc)} onCommit={(v) => write('mepc', v & ~1)} title="Exception program counter: where mret returns to" />
                  {c.mepc !== 0 && <span className="dim"> {symbols?.describeCode(c.mepc)}</span>}
                </td>
              </tr>
              <tr className="row-hot">
                <td className="dim">mcause</td>
                <td>
                  <EditableValue value={c.mcause} display={hex(c.mcause, 8)} max={U32} className={changed(c.mcause, b?.mcause)} onCommit={(v) => write('mcause', v)} title="Trap cause: bit 31 = interrupt, low bits = code" />
                  <span className="dim"> {c.mcause === 0 && c.mepc === 0 ? 'no trap yet' : cause.name}</span>
                </td>
              </tr>
              <tr className="row-hot">
                <td className="dim">mtval</td>
                <td>
                  <EditableValue value={c.mtval} display={hex(c.mtval, 8)} max={U32} className={changed(c.mtval, b?.mtval)} onCommit={(v) => write('mtval', v)} title="Trap value: faulting address, or the instruction bits for illegal instructions" />
                </td>
              </tr>
              <tr className="row-hot">
                <td className="dim">mscratch</td>
                <td>
                  <EditableValue value={c.mscratch} display={hex(c.mscratch, 8)} max={U32} className={changed(c.mscratch, b?.mscratch)} onCommit={(v) => write('mscratch', v)} title="Scratch register for trap handlers" />
                </td>
              </tr>
            </tbody>
          </table>
        </Section>
      </div>
    </div>
  );
}
