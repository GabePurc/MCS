import type { JSX } from 'react';
import { useSim, resetStopwatch } from '../state/sim';
import { sim } from '../services/simClient';
import { useWorkspace } from '../state/workspace';
import { formatHz, formatTime, hex } from '../format';
import { EditableValue, EmptyHint, Section } from './common';
import { Icons } from '../icons';

const FLAGS = ['I', 'T', 'H', 'S', 'V', 'N', 'Z', 'C'];
const FLAG_DESC: Record<string, string> = {
  I: 'Global Interrupt Enable', T: 'Bit Copy Storage', H: 'Half Carry', S: 'Sign (N xor V)', V: "Two's Complement Overflow", N: 'Negative', Z: 'Zero', C: 'Carry',
};
const SLEEP_MODES = ['Idle', 'ADC Noise Reduction', 'Power-down', 'Reserved', 'Standby', 'Reserved', 'Reserved', 'Reserved'];

export function ProcessorPanel(): JSX.Element {
  const st = useSim((s) => s.state);
  const base = useSim((s) => s.baseline);
  const spec = useSim((s) => s.spec);
  const stopwatch = useSim((s) => s.stopwatch);
  const symbols = useWorkspace((s) => s.build?.symbols);
  if (!st || !spec) return <EmptyHint>Build or import a program to see the processor state.</EmptyHint>;
  const rc = spec.coreName === 'AVRrc';
  const firstReg = rc ? 16 : 0;
  const changed = (a: number, b: number | null | undefined) => (b !== undefined && b !== null && a !== b ? "changed" : "");
  const ptr = (r: Uint8Array, lo: number) => r[lo] | (r[lo + 1] << 8);
  const swCycles = st.cycles - stopwatch.cycles;
  const swTime = st.timeSec - stopwatch.time;
  const rows: [string, JSX.Element | string, string?][] = [
    [
      'Program Counter',
      <EditableValue value={st.pc * 2} display={hex(st.pc * 2, 4)} max={spec.flashSize - 2} title="Byte address (double-click to edit)" className={changed(st.pc, base?.pc)} onCommit={(v) => sim({ type: 'writeCpu', field: 'pc', value: v & ~1 })} />,
      symbols?.describeCode(st.pc * 2),
    ],
    ['Stack Pointer', <EditableValue value={st.sp} display={hex(st.sp, 4)} max={0xffff} className={changed(st.sp, base?.sp)} onCommit={(v) => sim({ type: 'writeCpu', field: 'sp', value: v })} />],
    ['X Register', <span className={`mono ${changed(ptr(st.regs, 26), base && ptr(base.regs, 26))}`}>{hex(ptr(st.regs, 26), 4)}</span>],
    ['Y Register', <span className={`mono ${changed(ptr(st.regs, 28), base && ptr(base.regs, 28))}`}>{hex(ptr(st.regs, 28), 4)}</span>],
    ['Z Register', <span className={`mono ${changed(ptr(st.regs, 30), base && ptr(base.regs, 30))}`}>{hex(ptr(st.regs, 30), 4)}</span>],
    ['Cycle Counter', <span className="mono">{st.cycles.toLocaleString()}</span>],
    ['Instructions', <span className="mono">{st.instructions.toLocaleString()}</span>],
    ['Frequency', formatHz(st.hz)],
    [
      'Stop Watch',
      <span className="mono">
        {formatTime(swTime)} ({swCycles.toLocaleString()} cyc)
      </span>,
    ],
    ['Elapsed', <span className="mono">{formatTime(st.timeSec)}</span>],
    ['State', st.resetHeld ? 'Held in reset' : st.sleeping ? `Sleeping (${SLEEP_MODES[st.sleepMode]})` : st.running ? 'Running' : 'Stopped'],
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
        <Section title="Status Register (SREG)">
          <div className="sreg-row">
            {FLAGS.map((f, i) => {
              const bit = 7 - i;
              const on = (st.sreg >> bit) & 1;
              const was = base ? (base.sreg >> bit) & 1 : on;
              return (
                <button
                  key={f}
                  className={`flag-box${on ? ' on' : ''}${on !== was ? ' changed' : ''}`}
                  data-tip={`${f}: ${FLAG_DESC[f]} (click to toggle)`}
                  onClick={() => sim({ type: 'writeCpu', field: 'sreg', value: st.sreg ^ (1 << bit) })}
                >
                  <span className="flag-name">{f}</span>
                  <span className="flag-led" />
                </button>
              );
            })}
            <span className="mono sreg-hex">{hex(st.sreg)}</span>
          </div>
        </Section>
        <Section title="Registers">
          <div className="reg-grid">
            {Array.from({ length: 32 - firstReg }, (_, k) => {
              const r = firstReg + k;
              const v = st.regs[r];
              return (
                <div key={r} className="reg-cell">
                  <span className="reg-name">R{r}</span>
                  <EditableValue value={v} display={hex(v)} title={`R${r} = ${v} (${(v << 24) >> 24} signed)`} className={changed(v, base?.regs[r])} onCommit={(nv) => sim({ type: 'writeReg', reg: r, value: nv })} />
                </div>
              );
            })}
          </div>
        </Section>
      </div>
    </div>
  );
}
