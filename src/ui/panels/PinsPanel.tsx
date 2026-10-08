import { Fragment, useState, type JSX } from 'react';
import { useSim } from '../state/sim';
import { useSettings } from '../state/settings';
import { sim } from '../services/simClient';
import type { ExtDrive, PinGenerator, PinSpec, PinState } from '../backend/types';
import { formatHz, parseHz } from '../format';
import { Icons } from '../icons';
import { EmptyHint, Section } from './common';

const NEXT_DRIVE: Record<ExtDrive, ExtDrive> = { float: 'high', high: 'low', low: 'float', analog: 'float' };

function setDrive(pin: number, ext: ExtDrive, volts: number): void {
  sim({ type: 'setPin', pin, ext, volts });
}

function driverText(p: PinState): string {
  if (p.reserved) return p.reservedBy === 'RESET' || !p.reservedBy ? 'RESET input' : `${p.reservedBy} (clock)`;
  if (p.dir) return p.ovEnable ? 'Timer output' : 'PORT output';
  if (p.gen) return 'Signal generator';
  if (p.ext === 'analog') return 'Analog input';
  if (p.ext !== 'float') return 'External';
  return p.pullup ? 'Pull-up' : 'Floating';
}

function genText(g: PinGenerator): string {
  return `${formatHz(g.hz)}, ${Math.round(g.duty * 100)}% ${g.invert ? 'low' : 'high'}${g.count ? `, ${g.count} pulse${g.count > 1 ? 's' : ''}` : ''}`;
}

/** Square wave / pulse burst settings for one pin (event-driven generator in the simulator). */
function GeneratorRow({ pin, state, name }: { pin: number; state: PinState; name: string }): JSX.Element {
  const g = state.gen;
  const [freq, setFreq] = useState(g ? formatHz(g.hz) : '1 kHz');
  const [duty, setDuty] = useState(g ? Math.round(g.duty * 100) : 50);
  const [count, setCount] = useState(g?.count ? String(g.count) : '');
  const [invert, setInvert] = useState(g?.invert ?? false);
  const hz = parseHz(freq);
  const n = count.trim() ? Math.max(1, Math.floor(Number(count))) : undefined;
  const valid = hz > 0 && (n === undefined || Number.isFinite(n));
  const start = () => valid && sim({ type: 'setPinGenerator', pin, gen: { hz, duty: duty / 100, count: n, invert } });
  return (
    <tr className="gen-row">
      <td colSpan={5}>
        <div className="gen-editor">
          <Icons.Generator size={14} />
          <b>{name}</b>
          <span>Frequency</span>
          <input className="w7-input mono" style={{ width: 80 }} value={freq} onChange={(e) => setFreq(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && start()} data-tip="e.g. 1 Hz, 50 kHz, 1e6" />
          <span>Duty</span>
          <input type="range" className="w7-slider" min={1} max={99} value={duty} onChange={(e) => setDuty(Number(e.target.value))} style={{ width: 70 }} />
          <span className="mono">{duty}%</span>
          <span>Pulses</span>
          <input className="w7-input mono" style={{ width: 44 }} placeholder="all" value={count} onChange={(e) => setCount(e.target.value.replace(/[^0-9]/g, ''))} data-tip="Empty = continuous square wave; a number = burst of that many pulses" />
          <label className="w7-check" data-tip="Idle high, pulses go low (e.g. a button on a pull-up line)">
            <input type="checkbox" checked={invert} onChange={(e) => setInvert(e.target.checked)} /> Active low
          </label>
          <button className="w7-btn small default" disabled={!valid} onClick={start}><span>{g ? 'Update' : 'Start'}</span></button>
          {g && <button className="w7-btn small" onClick={() => sim({ type: 'setPinGenerator', pin, gen: null })}><span>Stop</span></button>}
          {g && <span className="dim">Running: {genText(g)}</span>}
        </div>
      </td>
    </tr>
  );
}

/** Package diagram + per-pin stimulus controls (logic levels, analog voltages, VCC). */
export function PinsPanel(): JSX.Element {
  const spec = useSim((s) => s.spec);
  const st = useSim((s) => s.state);
  const vcc = useSettings((s) => s.vcc);
  const [genOpen, setGenOpen] = useState<Set<number>>(new Set());
  if (!spec || !st) return <EmptyHint>No device loaded.</EmptyHint>;
  const toggleGen = (i: number) => setGenOpen((s) => {
    const n = new Set(s);
    if (n.has(i)) n.delete(i);
    else n.add(i);
    return n;
  });
  const gpioPins = spec.pins.filter((p) => p.kind === 'io' && p.gpio !== undefined);
  return (
    <div className="panel">
      <div className="panel-scroll">
        <Section title={`${spec.name} - ${spec.package}`}>
          <ChipDiagram pins={spec.pins} states={st.pins} name={spec.name} vcc={st.vcc} />
        </Section>
        <Section title="Pin stimulus">
          <table className="grid-table pin-table">
            <thead>
              <tr>
                <th>Pin</th>
                <th>Level</th>
                <th>Driven by</th>
                <th>External source</th>
                <th>Voltage</th>
              </tr>
            </thead>
            <tbody>
              {gpioPins.map((ps) => {
                const i = ps.gpio!;
                const p = st.pins[i];
                if (!p) return null;
                const showGen = genOpen.has(i) || !!p.gen;
                // Momentary push button: pulls toward the opposite of the idle level.
                const pressLevel: ExtDrive = p.pullup ? 'low' : 'high';
                return (
                  <Fragment key={ps.name}>
                  <tr className="row-hot">
                    <td data-tip={ps.functions.join(', ')}>
                      <b>{ps.name}</b> <span className="dim">({ps.number})</span>
                    </td>
                    <td>
                      <span className={`led ${p.level ? (p.dir ? 'led-green' : 'led-blue') : 'led-off'}`} /> {p.level}
                      {p.dir ? <span className="dim"> out</span> : <span className="dim"> in</span>}
                    </td>
                    <td className="dim">{driverText(p)}</td>
                    <td>
                      <span className="seg">
                        {(['float', 'low', 'high', 'analog'] as ExtDrive[]).map((d) => (
                          <button
                            key={d}
                            className={`seg-btn${p.ext === d ? ' on' : ''}`}
                            data-tip={{ float: 'Not connected (high impedance)', low: 'Drive LOW (0 V)', high: 'Drive HIGH (VCC)', analog: 'Analog voltage (for ADC / comparator)' }[d]}
                            onClick={() => setDrive(i, d, d === 'analog' ? p.extVolts || st.vcc / 2 : 0)}
                          >
                            {{ float: 'Z', low: '0', high: '1', analog: '~' }[d]}
                          </button>
                        ))}
                        <button
                          className={`seg-btn${showGen ? ' on' : ''}`}
                          data-tip={p.gen ? `Signal generator: ${genText(p.gen)}` : 'Signal generator (square wave / pulses)'}
                          onClick={() => (p.gen ? sim({ type: 'setPinGenerator', pin: i, gen: null }) : toggleGen(i))}
                        >
                          <Icons.Generator size={12} />
                        </button>
                      </span>
                      <button
                        className="w7-btn small push-btn"
                        data-tip={`Push button: hold to drive ${ps.name} ${pressLevel === 'low' ? 'LOW (to GND)' : 'HIGH (to VCC)'}, release to let go`}
                        onPointerDown={(e) => {
                          e.currentTarget.setPointerCapture(e.pointerId);
                          setDrive(i, pressLevel, 0);
                        }}
                        onPointerUp={() => setDrive(i, 'float', 0)}
                      >
                        <span>Push</span>
                      </button>
                      {p.ext === 'analog' && (
                        <input
                          type="range"
                          className="w7-slider"
                          min={0}
                          max={st.vcc}
                          step={0.01}
                          value={p.extVolts}
                          onChange={(e) => setDrive(i, 'analog', Number(e.target.value))}
                        />
                      )}
                    </td>
                    <td className="mono">{p.volts.toFixed(2)} V</td>
                  </tr>
                  {showGen && <GeneratorRow pin={i} state={p} name={ps.name} />}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
          {gpioPins.some((p) => st.pins[p.gpio!]?.dir && st.pins[p.gpio!]?.ext !== 'float' && st.pins[p.gpio!]?.ext !== 'analog') && (
            <div className="hint warn">An output pin is also driven externally - check the Output window for contention warnings.</div>
          )}
        </Section>
        <Section title="Supply">
          <div className="supply-row">
            <span>VCC</span>
            <input
              type="range"
              className="w7-slider"
              min={1.8}
              max={5.5}
              step={0.05}
              value={vcc}
              onChange={(e) => {
                const v = Number(e.target.value);
                useSettings.getState().set({ vcc: v });
                sim({ type: 'setVcc', volts: v });
              }}
            />
            <span className="mono">{st.vcc.toFixed(2)} V</span>
          </div>
        </Section>
      </div>
    </div>
  );
}

/** Dual-in-line style package drawing generated from the pin list. */
function ChipDiagram({ pins, states, name, vcc }: { pins: PinSpec[]; states: PinState[]; name: string; vcc: number }): JSX.Element {
  const n = pins.length;
  const half = Math.ceil(n / 2);
  const pitch = 34;
  const bodyW = 120;
  const bodyH = half * pitch + 16;
  const W = 360;
  const x0 = (W - bodyW) / 2;
  const y0 = 10;
  const pinColor = (p: PinSpec) => {
    if (p.kind === 'vcc') return 'url(#pin-vcc)';
    if (p.kind === 'gnd') return 'url(#pin-gnd)';
    if (p.kind === 'ref') return 'url(#pin-analog)';
    const s = states[p.gpio!];
    if (!s) return 'url(#pin-idle)';
    if (s.ext === 'analog' && !s.dir) return 'url(#pin-analog)';
    return s.level ? (s.dir ? 'url(#pin-high-out)' : 'url(#pin-high-in)') : 'url(#pin-idle)';
  };
  return (
    <svg className="chip-svg" viewBox={`0 0 ${W} ${bodyH + 22}`} width="100%" style={{ maxWidth: 420 }}>
      <defs>
        <linearGradient id="chip-body" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#4a5563" />
          <stop offset="0.5" stopColor="#2b333d" />
          <stop offset="1" stopColor="#1b2027" />
        </linearGradient>
        <linearGradient id="pin-idle" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#f2f4f6" /><stop offset="1" stopColor="#a8b0ba" /></linearGradient>
        <linearGradient id="pin-high-out" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#c6ffbe" /><stop offset="1" stopColor="#24a524" /></linearGradient>
        <linearGradient id="pin-high-in" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#d6ecff" /><stop offset="1" stopColor="#3d8fe0" /></linearGradient>
        <linearGradient id="pin-analog" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#fff2c4" /><stop offset="1" stopColor="#e3a21a" /></linearGradient>
        <linearGradient id="pin-vcc" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#ffd0c4" /><stop offset="1" stopColor="#d2462b" /></linearGradient>
        <linearGradient id="pin-gnd" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#9aa6b3" /><stop offset="1" stopColor="#3b4552" /></linearGradient>
      </defs>
      <rect x={x0} y={y0} width={bodyW} height={bodyH} rx="5" fill="url(#chip-body)" stroke="#11161c" />
      <circle cx={x0 + 12} cy={y0 + 12} r="4" fill="#5c6878" stroke="#11161c" strokeWidth="0.5" />
      <text x={W / 2} y={y0 + bodyH / 2 + 4} textAnchor="middle" fill="#cfd8e2" fontSize="13" fontFamily="var(--font-ui)">{name}</text>
      {pins.map((p, idx) => {
        const left = idx < half;
        const row = left ? idx : n - 1 - idx;
        const cy = y0 + 16 + row * pitch + pitch / 2 - 8;
        const lx = left ? x0 - 26 : x0 + bodyW;
        const s = p.gpio !== undefined ? states[p.gpio] : undefined;
        const interactive = p.kind === 'io' && s && !s.reserved;
        const label = p.kind === 'io' ? p.name : p.kind === 'vcc' ? `VCC ${vcc.toFixed(1)}V` : p.kind === 'ref' ? p.name : 'GND';
        const fns = p.functions.filter((f) => f !== p.name).slice(0, 3).join(' / ');
        return (
          <g
            key={p.number}
            className={interactive ? 'chip-pin interactive' : 'chip-pin'}
            onClick={() => interactive && setDrive(p.gpio!, NEXT_DRIVE[s!.ext], s!.extVolts)}
            data-tip={p.kind === 'io' ? `${p.name} (pin ${p.number}): ${p.functions.join(', ')}\nClick to cycle the external source: Z -> 1 -> 0` : label}
          >
            <rect x={lx} y={cy} width="26" height="14" rx="2" fill={pinColor(p)} stroke="#3b4552" strokeWidth="0.8" />
            <text x={left ? x0 + 8 : x0 + bodyW - 8} y={cy + 11} textAnchor={left ? 'start' : 'end'} fill="#e8eef5" fontSize="10.5" fontFamily="var(--font-mono)">{p.number}</text>
            <text x={left ? lx - 6 : lx + 32} y={cy + 6} textAnchor={left ? 'end' : 'start'} fontSize="12" fontWeight="600" fill="#1e395b" fontFamily="var(--font-ui)">{label}</text>
            <text x={left ? lx - 6 : lx + 32} y={cy + 18} textAnchor={left ? 'end' : 'start'} fontSize="9.5" fill="#6d7a8a" fontFamily="var(--font-ui)">{fns}</text>
            {s && s.dir === 1 && (
              <path d={left ? `M${lx - 2} ${cy + 7} l-6 -4 v8z` : `M${lx + 28} ${cy + 7} l6 -4 v8z`} fill="#24a524" />
            )}
          </g>
        );
      })}
    </svg>
  );
}
