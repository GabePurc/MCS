import { useEffect, useRef, useState, type JSX } from 'react';
import { trace } from '../state/trace';
import { useSim } from '../state/sim';
import { Icons } from '../icons';
import { formatHz, formatTime } from '../format';
import { EmptyHint } from './common';
import { gpioNames } from '../services/device';

/** View state survives tab switches. */
const view = { start: 0, cyclesPerPx: 200, follow: true, cursorA: -1, cursorB: -1 };

const ROW_H = 34;
const HEADER_H = 22;
const LABEL_W = 64;

/**
 * Logic analyzer: draws every pin transition recorded by the simulator on a canvas. Wheel to
 * zoom (around the mouse), drag to pan, click to place cursor A, Shift+click for cursor B.
 */
export function WaveformPanel(): JSX.Element {
  const spec = useSim((s) => s.spec);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [, setTick] = useState(0);
  const rerender = () => setTick((t) => t + 1);
  const names = spec ? gpioNames(spec) : [];

  useEffect(() => {
    const canvas = canvasRef.current;
    const wrap = wrapRef.current;
    if (!canvas || !wrap) return;
    let raf = 0;
    const draw = () => {
      raf = 0;
      drawWave(canvas, names);
    };
    const schedule = () => {
      if (!raf) raf = requestAnimationFrame(draw);
    };
    const ro = new ResizeObserver(schedule);
    ro.observe(wrap);
    const unsub = trace.subscribe(schedule);
    schedule();
    return () => {
      ro.disconnect();
      unsub();
      cancelAnimationFrame(raf);
    };
  }, [names.join()]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!spec) return <EmptyHint>No device loaded.</EmptyHint>;

  const redraw = () => {
    if (canvasRef.current) drawWave(canvasRef.current, names);
    rerender();
  };
  const zoom = (factor: number, anchorPx?: number) => {
    const w = (canvasRef.current?.clientWidth ?? 600) - LABEL_W;
    const ax = anchorPx ?? w / 2;
    const anchorCycle = view.start + ax * view.cyclesPerPx;
    view.cyclesPerPx = Math.min(1e9, Math.max(0.02, view.cyclesPerPx * factor));
    view.start = Math.max(0, anchorCycle - ax * view.cyclesPerPx);
    if (factor < 1 || anchorPx !== undefined) view.follow = view.follow && anchorPx === undefined;
    redraw();
  };
  const fit = () => {
    const w = (canvasRef.current?.clientWidth ?? 600) - LABEL_W;
    const first = trace.count ? trace.cycleAt(0) : 0;
    const span = Math.max(1, trace.endCycle - first);
    view.cyclesPerPx = span / Math.max(50, w - 10);
    view.start = first;
    view.follow = false;
    redraw();
  };

  const hz = trace.hz || 1;
  const dt = view.cursorA >= 0 && view.cursorB >= 0 ? Math.abs(view.cursorB - view.cursorA) / hz : null;

  return (
    <div className="panel">
      <div className="panel-toolbar">
        <button className={`tb-btn${view.follow ? ' pressed' : ''}`} data-tip="Follow live signal" onClick={() => { view.follow = !view.follow; redraw(); }}>
          <Icons.Follow /> Follow
        </button>
        <button className="tb-btn" data-tip="Zoom in" onClick={() => zoom(0.5)}><Icons.ZoomIn /></button>
        <button className="tb-btn" data-tip="Zoom out" onClick={() => zoom(2)}><Icons.ZoomOut /></button>
        <button className="tb-btn" data-tip="Zoom to fit" onClick={fit}><Icons.ZoomFit /></button>
        <button className="tb-btn" data-tip="Clear captured signals" onClick={() => { trace.clear(); view.cursorA = view.cursorB = -1; redraw(); }}><Icons.Clear /></button>
        <span className="tb-sep" />
        <span className="dim">{formatTime(view.cyclesPerPx / hz)}/px</span>
        {view.cursorA >= 0 && <span className="wave-readout">A: {formatTime(view.cursorA / hz)}</span>}
        {view.cursorB >= 0 && <span className="wave-readout">B: {formatTime(view.cursorB / hz)}</span>}
        {dt !== null && dt > 0 && (
          <span className="wave-readout strong">
            Δt = {formatTime(dt)} ({formatHz(1 / dt)})
          </span>
        )}
      </div>
      <div className="wave-wrap" ref={wrapRef}>
        <canvas
          ref={canvasRef}
          onWheel={(e) => {
            const rect = e.currentTarget.getBoundingClientRect();
            const x = e.clientX - rect.left - LABEL_W;
            if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
              view.start = Math.max(0, view.start + (e.deltaX || e.deltaY) * view.cyclesPerPx);
              view.follow = false;
              redraw();
            } else zoom(e.deltaY > 0 ? 1.25 : 0.8, Math.max(0, x));
          }}
          onPointerDown={(e) => {
            const el = e.currentTarget;
            const rect = el.getBoundingClientRect();
            const sx = e.clientX;
            const start0 = view.start;
            let moved = false;
            el.setPointerCapture(e.pointerId);
            const move = (ev: PointerEvent) => {
              const dx = ev.clientX - sx;
              if (Math.abs(dx) > 3) moved = true;
              if (moved) {
                view.start = Math.max(0, start0 - dx * view.cyclesPerPx);
                view.follow = false;
                drawWave(el, names);
              }
            };
            const up = (ev: PointerEvent) => {
              el.removeEventListener('pointermove', move);
              el.removeEventListener('pointerup', up);
              if (!moved) {
                const x = ev.clientX - rect.left - LABEL_W;
                if (x >= 0) {
                  const c = view.start + x * view.cyclesPerPx;
                  if (ev.shiftKey) view.cursorB = c;
                  else view.cursorA = c;
                }
              }
              redraw();
            };
            el.addEventListener('pointermove', move);
            el.addEventListener('pointerup', up);
          }}
        />
      </div>
    </div>
  );
}

function niceStep(raw: number): number {
  const p = Math.pow(10, Math.floor(Math.log10(raw)));
  for (const m of [1, 2, 5, 10]) if (m * p >= raw) return m * p;
  return 10 * p;
}

function drawWave(canvas: HTMLCanvasElement, names: string[]): void {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.parentElement!.clientWidth;
  const h = Math.max(canvas.parentElement!.clientHeight, HEADER_H + names.length * ROW_H + 4);
  if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;
  }
  const ctx = canvas.getContext('2d')!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  const plotW = w - LABEL_W;
  if (view.follow) view.start = Math.max(0, trace.endCycle - plotW * view.cyclesPerPx * 0.95);
  const start = view.start;
  const end = start + plotW * view.cyclesPerPx;
  const hz = trace.hz || 1;

  // Background
  ctx.fillStyle = '#0f1b25';
  ctx.fillRect(0, 0, w, h);
  ctx.fillStyle = '#e9eef5';
  ctx.fillRect(0, 0, LABEL_W, h);

  // Time axis
  const header = ctx.createLinearGradient(0, 0, 0, HEADER_H);
  header.addColorStop(0, '#ffffff');
  header.addColorStop(1, '#e3eaf4');
  ctx.fillStyle = header;
  ctx.fillRect(0, 0, w, HEADER_H);
  ctx.strokeStyle = '#a5b4c8';
  ctx.beginPath();
  ctx.moveTo(0, HEADER_H + 0.5);
  ctx.lineTo(w, HEADER_H + 0.5);
  ctx.stroke();
  const secPerPx = view.cyclesPerPx / hz;
  const step = niceStep(secPerPx * 90);
  const t0 = Math.ceil(start / hz / step) * step;
  ctx.font = '11px ' + getComputedStyle(document.body).getPropertyValue('--font-ui');
  ctx.textBaseline = 'middle';
  for (let t = t0; t * hz <= end; t += step) {
    const x = LABEL_W + (t * hz - start) / view.cyclesPerPx;
    ctx.strokeStyle = 'rgba(120,150,180,0.25)';
    ctx.beginPath();
    ctx.moveTo(Math.round(x) + 0.5, HEADER_H);
    ctx.lineTo(Math.round(x) + 0.5, h);
    ctx.stroke();
    ctx.strokeStyle = '#7a8ea8';
    ctx.beginPath();
    ctx.moveTo(Math.round(x) + 0.5, HEADER_H - 5);
    ctx.lineTo(Math.round(x) + 0.5, HEADER_H);
    ctx.stroke();
    ctx.fillStyle = '#1e395b';
    ctx.fillText(formatTime(t), x + 3, 9);
  }

  // Rows
  for (let r = 0; r < names.length; r++) {
    const top = HEADER_H + r * ROW_H;
    const yHi = top + 8;
    const yLo = top + ROW_H - 8;
    ctx.fillStyle = r % 2 ? '#122230' : '#0f1b25';
    ctx.fillRect(LABEL_W, top, plotW, ROW_H);
    ctx.fillStyle = '#1e395b';
    ctx.fillText(names[r], 8, top + ROW_H / 2);
    ctx.strokeStyle = '#c5d2e2';
    ctx.beginPath();
    ctx.moveTo(0, top + ROW_H + 0.5);
    ctx.lineTo(LABEL_W, top + ROW_H + 0.5);
    ctx.stroke();
    if (trace.count === 0) continue;

    const color = ['#5df07b', '#ffd23b', '#62c4ff', '#ff8a65', '#c792ea', '#80cbc4', '#f48fb1', '#e6ee9c'][r % 8];
    ctx.strokeStyle = color;
    ctx.fillStyle = color + '55';
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    let i = Math.max(0, trace.indexAt(start));
    let level = trace.bitAt(i, r);
    let x = LABEL_W;
    const xEnd = LABEL_W + Math.min(plotW, (Math.min(end, trace.endCycle) - start) / view.cyclesPerPx);
    if (trace.cycleAt(0) > start) x = LABEL_W + (trace.cycleAt(0) - start) / view.cyclesPerPx;
    ctx.moveTo(x, level ? yHi : yLo);
    let lastPx = -1;
    let denseFrom = -1;
    for (i = i + 1; i < trace.count; i++) {
      const c = trace.cycleAt(i);
      if (c > end) break;
      const nl = trace.bitAt(i, r);
      if (nl === level) continue;
      const px = Math.round(LABEL_W + (c - start) / view.cyclesPerPx);
      if (px === lastPx) {
        // Several edges inside one pixel: draw a filled activity block instead.
        if (denseFrom < 0) denseFrom = px;
        level = nl;
        continue;
      }
      if (denseFrom >= 0) {
        ctx.fillRect(denseFrom, yHi, Math.max(1, lastPx - denseFrom + 1), yLo - yHi);
        ctx.moveTo(lastPx, level ? yHi : yLo);
        denseFrom = -1;
      }
      ctx.lineTo(px, level ? yHi : yLo);
      ctx.lineTo(px, nl ? yHi : yLo);
      level = nl;
      lastPx = px;
    }
    if (denseFrom >= 0) ctx.fillRect(denseFrom, yHi, Math.max(1, lastPx - denseFrom + 1), yLo - yHi);
    ctx.lineTo(Math.max(xEnd, lastPx), level ? yHi : yLo);
    ctx.stroke();
    ctx.lineWidth = 1;
  }

  // Cursors
  const cursor = (c: number, col: string, label: string) => {
    if (c < start || c > end) return;
    const x = Math.round(LABEL_W + (c - start) / view.cyclesPerPx) + 0.5;
    ctx.strokeStyle = col;
    ctx.setLineDash([4, 3]);
    ctx.beginPath();
    ctx.moveTo(x, HEADER_H);
    ctx.lineTo(x, h);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = col;
    ctx.fillText(label, x + 3, HEADER_H + 8);
  };
  cursor(view.cursorA, '#ff5252', 'A');
  cursor(view.cursorB, '#40c4ff', 'B');
}
