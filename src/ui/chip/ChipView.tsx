/**
 * Chip View: the inside of the microcontroller while the program runs. A 3D model (package,
 * lead frame, bond wires, die) or a flat die view, both showing the live state on the silicon:
 * flash execution heat map and PC, SRAM bytes and stack, register file, the instruction being
 * decoded, SREG, peripheral activity and pin levels. Slow the simulation down (Speed: 1 Hz...)
 * to watch every instruction.
 */
import { useEffect, useMemo, useRef, useState, type JSX } from 'react';
import { disassemble } from '../backend/api';
import { Icons } from '../icons';
import { useSim } from '../state/sim';
import { useSettings } from '../state/settings';
import { useWorkspace } from '../state/workspace';
import { useLayout, type PanelId } from '../state/layout';
import { openDialog } from '../state/dialogs';
import { loadJson, saveJson } from '../state/persist';
import { sim } from '../services/simClient';
import { speedLabel } from '../services/commands';
import { hex } from '../format';
import { EmptyHint } from '../panels/common';
import { buildFloorplan, type Block, type Pad } from './floorplan';
import { BlockLayers, Die2D, LiveModel, layerResolution, type Hover } from './engine';
import { Chip3D, type Shell } from './scene3d';

type Mode = '3d' | '2d';
const KEY = 'mcs.chip.v1';

const OPEN: Partial<Record<Block['kind'], PanelId>> = {
  flash: 'disasm', decoder: 'disasm', sram: 'memory', eeprom: 'memory', regs: 'processor', alu: 'processor', control: 'processor',
  iobus: 'io', periph: 'io', analog: 'io', nvm: 'io',
};

function describe(h: Hover['hit']): { title: string; lines: string[] } | null {
  if (!h) return null;
  const st = useSim.getState().state;
  if (h.pad) {
    const p: Pad = h.pad;
    const ps = st && p.pin.gpio !== undefined ? st.pins[p.pin.gpio] : undefined;
    const lines = [`Package pin ${p.pin.number}${p.pin.functions.length ? ` - ${p.pin.functions.join(', ')}` : ''}`];
    if (ps) lines.push(`Level ${ps.level} (${ps.volts.toFixed(2)} V), ${ps.dir ? 'output' : 'input'}${ps.gen ? ', signal generator attached' : ''}`);
    if (p.pin.kind === 'io') lines.push('Click: Pins & Stimulus');
    return { title: `Bond pad ${p.pin.name}`, lines };
  }
  const b = h.block!;
  const lines = [b.sub];
  if (st) {
    if (b.kind === 'flash') lines.push(`PC = ${hex(st.pc * 2, 4)} (word ${st.pc}). Bright cells ran recently.`);
    if (b.kind === 'sram') lines.push(`SP = ${hex(st.sp, 4)}. Orange = just written, blue = stack.`);
    if (b.kind === 'control') lines.push(`${st.cycles.toLocaleString()} cycles, ${st.instructions.toLocaleString()} instructions`);
  }
  const target = b.kind === 'clock' ? 'Supply & Clock' : OPEN[b.kind] ? { disasm: 'Disassembly', memory: 'Memory', processor: 'Processor', io: 'I/O View' }[OPEN[b.kind] as string] : null;
  if (target) lines.push(`Click: ${target}`);
  return { title: b.label, lines };
}

export function ChipView(): JSX.Element {
  const spec = useSim((s) => s.spec);
  const speedMode = useSettings((s) => s.speedMode);
  const speedFactor = useSettings((s) => s.speedFactor);
  const saved = useMemo(() => loadJson<{ mode: Mode; shell: Shell; shading: boolean }>(KEY, { mode: '3d', shell: 'xray', shading: true }), []);
  const [mode, setModeState] = useState<Mode>(saved.mode);
  const [shell, setShellState] = useState<Shell>(saved.shell);
  const [shading, setShadingState] = useState(saved.shading);
  const [hover, setHover] = useState<Hover | null>(null);
  const [error, setError] = useState<string | null>(null);
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<Chip3D | Die2D | null>(null);
  const setMode = (m: Mode) => {
    setModeState(m);
    saveJson(KEY, { mode: m, shell, shading });
  };
  const setShell = (s: Shell) => {
    setShellState(s);
    saveJson(KEY, { mode, shell: s, shading });
    if (view.current instanceof Chip3D) view.current.setShell(s);
  };
  const setShading = (on: boolean) => {
    setShadingState(on);
    saveJson(KEY, { mode, shell, shading: on });
    if (view.current instanceof Chip3D) view.current.setShading(on);
  };

  const plan = useMemo(() => (spec ? buildFloorplan(spec) : null), [spec]);
  const live = useMemo(() => (spec && plan ? { model: new LiveModel(spec), layers: new BlockLayers(plan, layerResolution(plan)) } : null), [spec, plan]);

  // Execution profiling feeds the flash heat map while the view is open.
  useEffect(() => {
    if (!spec) return;
    sim({ type: 'setProfiling', enabled: true });
    return () => sim({ type: 'setProfiling', enabled: false });
  }, [spec]);

  // Machine state -> live model -> block canvases -> renderer.
  useEffect(() => {
    if (!spec || !live) return;
    const { model, layers } = live;
    let lastKey = '';
    const feed = () => {
      const s = useSim.getState();
      if (!s.state) return;
      layers.update(model.data(s.state, s.running), model.heatVersion);
      view.current?.setState(s.state, s.state.vcc);
    };
    const loadDisasm = () => {
      const flash = useSim.getState().flash;
      const key = `${spec.id}:${flash?.length}:${useSim.getState().state?.flashVersion}`;
      if (!flash || key === lastKey) return;
      lastKey = key;
      const labels: Record<number, string> = {};
      for (const sym of useWorkspace.getState().build?.symbols.code ?? []) if (!(sym.address in labels)) labels[sym.address] = sym.name;
      disassemble(spec.id, flash, labels)
        .then((lines) => {
          model.disasm = new Map(lines.map((l) => [l.pc, l.operands ? `${l.mnemonic} ${l.operands}` : l.mnemonic]));
          feed();
        })
        .catch(() => {});
    };
    model.flash = useSim.getState().flash;
    const st0 = useSim.getState().state;
    if (st0) model.update(st0);
    loadDisasm();
    feed();
    return useSim.subscribe((s, p) => {
      if (s.flash !== p.flash) {
        model.flash = s.flash;
        loadDisasm();
      }
      if (s.state && s.state !== p.state) {
        model.update(s.state);
        feed();
      }
    });
  }, [spec, live]);

  // Renderer (re)creation.
  useEffect(() => {
    const el = host.current;
    if (!el || !spec || !plan || !live) return;
    const click = (h: Hover) => {
      if (h.hit?.pad) useLayout.getState().show('pins');
      else if (h.hit?.block?.kind === 'clock') openDialog('supply');
      else if (h.hit?.block && OPEN[h.hit.block.kind]) useLayout.getState().show(OPEN[h.hit.block.kind]!);
    };
    let v: Chip3D | Die2D;
    let canvas: HTMLCanvasElement | null = null;
    try {
      if (mode !== '3d') throw new Error('flat');
      v = new Chip3D(el, plan, spec, live.layers, setHover, click);
      v.setShell(shell);
      v.setShading(shading);
      setError(null);
    } catch (e) {
      if (mode === '3d') setError(`3D view unavailable (${e instanceof Error ? e.message : String(e)}); showing the flat die view.`);
      canvas = document.createElement('canvas');
      canvas.className = 'chip-canvas';
      el.appendChild(canvas);
      v = new Die2D(canvas, plan, spec, live.layers, setHover, click);
    }
    view.current = v;
    // Every layer must reach the new renderer's textures.
    for (const l of live.layers.layers) l.dirty = true;
    const st = useSim.getState().state;
    if (st) v.setState(st, st.vcc);
    return () => {
      view.current = null;
      v.dispose();
      canvas?.remove();
    };
  }, [mode, spec, plan, live]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!spec) return <EmptyHint>No device loaded.</EmptyHint>;
  const tip = hover ? describe(hover.hit) : null;
  return (
    <div className="panel chip-view">
      <div className="panel-toolbar">
        <span className="seg">
          <button className={`seg-btn${mode === '3d' ? ' on' : ''}`} onClick={() => setMode('3d')} data-tip="3D model: drag to orbit, right-drag to pan, wheel to zoom">3D</button>
          <button className={`seg-btn${mode === '2d' ? ' on' : ''}`} onClick={() => setMode('2d')} data-tip="Flat die view: wheel to zoom, drag to pan, double-click to fit">Flat</button>
        </span>
        {mode === '3d' && !error && (
          <>
            <span>Package:</span>
            <span className="seg">
              {(['xray', 'solid', 'off'] as Shell[]).map((s) => (
                <button key={s} className={`seg-btn${shell === s ? ' on' : ''}`} onClick={() => setShell(s)}>{{ xray: 'X-ray', solid: 'Solid', off: 'Hidden' }[s]}</button>
              ))}
            </span>
            <label className="w7-check" data-tip="Soft shadows and ambient occlusion (turn off on slow graphics hardware)">
              <input type="checkbox" checked={shading} onChange={(e) => setShading(e.target.checked)} /> Shading
            </label>
            <button className="w7-btn small" onClick={() => (view.current as Chip3D | null)?.topView()} data-tip="Look straight down at the die to read the live values"><span>Die close-up</span></button>
          </>
        )}
        <button className="w7-btn small" onClick={() => view.current?.resetView()} data-tip="Reset the camera"><Icons.ZoomFit size={13} /><span>Reset</span></button>
        <div className="grow" />
        <button className="w7-btn small" onClick={() => openDialog('speed')} data-tip="Slow the CPU down (down to 1 Hz) to watch each instruction">
          <span>Speed: {speedLabel(speedMode, speedFactor)}</span>
        </button>
      </div>
      <div className="chip-stage" ref={host}>
        {error && <div className="chip-error">{error}</div>}
        <div className="chip-legend">
          <span><i className="lg-heat" /> executed recently</span>
          <span><i className="lg-pc" /> PC</span>
          <span><i className="lg-write" /> written</span>
          <span><i className="lg-stack" /> stack</span>
          <span><i className="lg-act" /> peripheral active</span>
          <span className="dim">{plan && spec.die ? `Die ${(spec.die.widthUm / 1000).toFixed(2)} x ${(spec.die.heightUm / 1000).toFixed(2)} mm, illustrative floorplan` : 'Illustrative floorplan'}</span>
        </div>
        {tip && hover && (
          <div className="chip-tip" style={{ left: Math.min(hover.x + 14, (host.current?.clientWidth ?? 400) - 260), top: hover.y + 16 }}>
            <b>{tip.title}</b>
            {tip.lines.map((l, i) => <div key={i}>{l}</div>)}
          </div>
        )}
      </div>
    </div>
  );
}
