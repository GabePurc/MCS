import type { JSX } from 'react';
import { armCore, armHasDouble, armHasFpu, type CpuField } from '../backend/types';
import { useSim, resetStopwatch } from '../state/sim';
import { sim } from '../services/simClient';
import { useWorkspace } from '../state/workspace';
import { f32FromBits, f64FromBits, formatFloat, formatHz, formatTime, hex } from '../format';
import { EditableValue, EmptyHint, Flag, Section } from './common';
import { Icons } from '../icons';
import { useState } from 'react';
import { activeStack, exceptionName } from '../services/armState';

const U32 = 0xffffffff;
const changed = (a: number | boolean, b: number | boolean | undefined) => (b !== undefined && a !== b ? 'changed' : '');

// xPSR bits
const XPSR_FLAGS: [string, number, string][] = [
  ['N', 31, 'Negative'],
  ['Z', 30, 'Zero'],
  ['C', 29, 'Carry / not borrow'],
  ['V', 28, 'Overflow'],
  ['Q', 27, 'Saturation (sticky)'],
];
const CONTROL_BITS: [string, number, string][] = [
  ['nPRIV', 0, 'Thread mode is unprivileged'],
  ['SPSEL', 1, 'Thread mode uses PSP'],
  ['FPCA', 2, 'Floating-point context active'],
];
const FPSCR_FLAGS: [string, number, string][] = [
  ['N', 31, 'Negative (compare)'],
  ['Z', 30, 'Zero (compare)'],
  ['C', 29, 'Carry (compare)'],
  ['V', 28, 'Unordered / overflow (compare)'],
  ['AHP', 26, 'Alternative half-precision format'],
  ['DN', 25, 'Default NaN mode'],
  ['FZ', 24, 'Flush-to-zero mode'],
  ['IDC', 7, 'Input denormal cumulative exception'],
  ['IXC', 4, 'Inexact cumulative exception'],
  ['UFC', 3, 'Underflow cumulative exception'],
  ['OFC', 2, 'Overflow cumulative exception'],
  ['DZC', 1, 'Division by zero cumulative exception'],
  ['IOC', 0, 'Invalid operation cumulative exception'],
];
const RMODES = ['to nearest', 'toward +inf', 'toward -inf', 'toward zero'];
const REG_NAMES = ['R0', 'R1', 'R2', 'R3', 'R4', 'R5', 'R6', 'R7', 'R8', 'R9', 'R10', 'R11', 'R12'];

function write(field: CpuField, value: number): void {
  sim({ type: 'writeCpu', field, value: value >>> 0 });
}

/** Cortex-M processor view: registers, program status, special registers and the FPU. */
export function ArmProcessorPanel(): JSX.Element {
  const st = useSim((s) => s.state);
  const base = useSim((s) => s.baseline);
  const spec = useSim((s) => s.spec);
  const stopwatch = useSim((s) => s.stopwatch);
  const symbols = useWorkspace((s) => s.build?.symbols);
  const [dView, setDView] = useState(false);
  if (!st || !spec || spec.arch !== 'arm' || st.core.arch !== 'arm') return <EmptyHint>Load a program to see the processor state.</EmptyHint>;
  const c = armCore(st);
  const b = base && base.core.arch === 'arm' ? base.core : null;
  const swCycles = st.cycles - stopwatch.cycles;
  const swTime = st.timeSec - stopwatch.time;
  const active = activeStack(c);
  const ipsr = c.xpsr & 0x1ff;
  const itState = (((c.xpsr >>> 10) & 0x3f) << 2) | ((c.xpsr >>> 25) & 3);
  const ge = (c.xpsr >>> 16) & 0xf;
  const fpu = armHasFpu(spec);
  const rows: [string, JSX.Element | string, string?][] = [
    [
      'Program Counter',
      <EditableValue value={st.pc} display={hex(st.pc, 8)} max={U32} title="Byte address (double-click to edit)" className={changed(st.pc, base?.pc)} onCommit={(v) => write('pc', v & ~1)} />,
      symbols?.describeCode(st.pc),
    ],
    ['Stack Pointer', <EditableValue value={c.r[13]} display={hex(c.r[13], 8)} max={U32} title={`Active stack pointer (${active})`} className={changed(c.r[13], b?.r[13])} onCommit={(v) => write('sp', v & ~3)} />, active],
    ['Main SP (MSP)', <EditableValue value={c.msp} display={hex(c.msp, 8)} max={U32} className={changed(c.msp, b?.msp)} onCommit={(v) => write('msp', v & ~3)} />, active === 'MSP' ? '(active)' : undefined],
    ['Process SP (PSP)', <EditableValue value={c.psp} display={hex(c.psp, 8)} max={U32} className={changed(c.psp, b?.psp)} onCommit={(v) => write('psp', v & ~3)} />, active === 'PSP' ? '(active)' : undefined],
    ['Link Register', <EditableValue value={c.r[14]} display={hex(c.r[14], 8)} max={U32} className={changed(c.r[14], b?.r[14])} onCommit={(v) => write('lr', v)} />, c.r[14] >= 0xf0000000 ? 'EXC_RETURN' : c.r[14] ? symbols?.describeCode(c.r[14] & ~1) : undefined],
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
    ['State', st.sleeping ? 'Sleeping (WFI/WFE)' : st.running ? 'Running' : 'Stopped'],
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
            {REG_NAMES.map((name, r) => (
              <div key={r} className="reg-cell">
                <span className="reg-name">{name}</span>
                <EditableValue value={c.r[r]} display={hex(c.r[r], 8)} max={U32} title={`${name} = ${c.r[r]} (${c.r[r] | 0} signed)`} className={changed(c.r[r], b?.r[r])} onCommit={(v) => sim({ type: 'writeReg', reg: r, value: v >>> 0 })} />
              </div>
            ))}
          </div>
        </Section>
        <Section title="Program Status (xPSR)">
          <div className="psr-row">
            {XPSR_FLAGS.map(([n, bit, tip]) => (
              <Flag key={n} name={n} on={((c.xpsr >>> bit) & 1) === 1} was={b ? ((b.xpsr >>> bit) & 1) === 1 : undefined} tip={tip} onClick={() => write('xpsr', c.xpsr ^ (1 << bit))} />
            ))}
            <span className="dim">GE</span>
            {[3, 2, 1, 0].map((i) => (
              <Flag key={i} name={`${i}`} on={((ge >> i) & 1) === 1} was={b ? ((((b.xpsr >>> 16) & 0xf) >> i) & 1) === 1 : undefined} tip={`Greater-than-or-equal flag for byte ${i} (SIMD)`} onClick={() => write('xpsr', c.xpsr ^ (1 << (16 + i)))} />
            ))}
            <span className="mono sreg-hex">{hex(c.xpsr, 8)}</span>
          </div>
          <div className="psr-info">
            IPSR <b className="mono">{ipsr}</b> <span className="dim">{exceptionName(spec, ipsr)}</span>
            {' · '}EPSR T=<b className="mono">{(c.xpsr >>> 24) & 1}</b> IT/ICI=<b className="mono">{hex(itState, 2)}</b>
          </div>
        </Section>
        <Section title="Special Registers">
          <div className="psr-row">
            <span className="dim">CONTROL</span>
            {CONTROL_BITS.filter(([n]) => n !== 'FPCA' || fpu).map(([n, bit, tip]) => (
              <Flag key={n} name={n} on={((c.control >> bit) & 1) === 1} was={b ? ((b.control >> bit) & 1) === 1 : undefined} tip={tip} onClick={() => write('control', c.control ^ (1 << bit))} />
            ))}
            <span className="dim">Masks</span>
            <Flag name="PRIMASK" on={c.primask} was={b?.primask} tip="Masks all configurable-priority exceptions" onClick={() => write('primask', c.primask ? 0 : 1)} />
            <Flag name="FAULTMASK" on={c.faultmask} was={b?.faultmask} tip="Masks all exceptions except NMI" onClick={() => write('faultmask', c.faultmask ? 0 : 1)} />
          </div>
          <div className="psr-info">
            BASEPRI <EditableValue value={c.basepri} display={hex(c.basepri, 2)} max={0xff} className={changed(c.basepri, b?.basepri)} onCommit={(v) => write('basepri', v)} title="Masks exceptions with priority >= value (0 = off)" />
            <span className="dim"> ({spec.nvicPrioBits} priority bits)</span>
          </div>
        </Section>
        {fpu && c.fpr.length === 32 && (
          <>
            <Section
              title={armHasDouble(spec) ? 'Floating Point (FPv5-D16)' : 'Floating Point (FPv4-SP)'}
              right={
                armHasDouble(spec) ? (
                  <button className={`w7-btn small${dView ? ' default' : ''}`} data-tip="Switch between single (S0-S31) and double (D0-D15) precision views" onClick={() => setDView(!dView)}>
                    <span>{dView ? 'D view' : 'S view'}</span>
                  </button>
                ) : undefined
              }
            >
              <div className="psr-row">
                <span className="dim">FPSCR</span>
                {FPSCR_FLAGS.map(([n, bit, tip]) => (
                  <Flag key={n} name={n} on={((c.fpscr >>> bit) & 1) === 1} was={b ? ((b.fpscr >>> bit) & 1) === 1 : undefined} tip={tip} onClick={() => write('fpscr', c.fpscr ^ (1 << bit))} />
                ))}
                <span className="mono sreg-hex">{hex(c.fpscr, 8)}</span>
              </div>
              <div className="psr-info">
                Rounding: <b>{RMODES[(c.fpscr >>> 22) & 3]}</b>
              </div>
              <div className="reg-grid fp">
                {dView && armHasDouble(spec)
                  ? Array.from({ length: 16 }, (_, d) => {
                      const lo = c.fpr[2 * d];
                      const hi = c.fpr[2 * d + 1];
                      return (
                        <div key={d} className="reg-cell" data-tip={`D${d} = S${2 * d} (low) : S${2 * d + 1} (high) - edit the S view to change it`}>
                          <span className="reg-name">D{d}</span>
                          <span className={`mono ${changed(lo, b?.fpr[2 * d])}${changed(hi, b?.fpr[2 * d + 1]) ? ' changed' : ''}`}>{hex(hi, 8)}{hexRaw8(lo)}</span>
                          <span className="reg-extra mono">{formatFloat(f64FromBits(lo, hi), 12)}</span>
                        </div>
                      );
                    })
                  : Array.from({ length: 32 }, (_, i) => (
                      <div key={i} className="reg-cell">
                        <span className="reg-name">S{i}</span>
                        <EditableValue value={c.fpr[i]} display={hex(c.fpr[i], 8)} max={U32} title={`S${i} = ${formatFloat(f32FromBits(c.fpr[i]), 9)} (double-click to edit the raw bits)`} className={changed(c.fpr[i], b?.fpr[i])} onCommit={(v) => sim({ type: 'writeReg', reg: 16 + i, value: v >>> 0 })} />
                        <span className="reg-extra mono">{formatFloat(f32FromBits(c.fpr[i]))}</span>
                      </div>
                    ))}
              </div>
            </Section>
          </>
        )}
      </div>
    </div>
  );
}

function hexRaw8(v: number): string {
  return (v >>> 0).toString(16).toUpperCase().padStart(8, '0');
}
