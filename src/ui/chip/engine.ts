/**
 * Chip View engine shared by the 2D and 3D renderers:
 * * `LiveModel` turns the stream of machine states into decaying highlights (execution heat per
 *   flash word, recent writes per data address and register, activity per peripheral group).
 * * `BlockLayers` keeps one canvas per die block with its live contents and only redraws the
 *   blocks whose visible content changed (each one doubles as a 3D texture).
 * * `Die2D` draws the die flat on a canvas with zoom and pan.
 */
import type { AvrDeviceSpec, MachineState } from '../backend/types';
import { blockSignature, drawBlockLive, drawDieBase, drawMemoryDetail, memoryGrid, pinColor, type LiveData, type MemGrid } from './dieArt';
import { hitTest, type Block, type Floorplan } from './floorplan';

const HEAT_DECAY = 0.82;
const WRITE_DECAY = 0.72;

export class LiveModel {
  readonly heat: Float32Array;
  readonly writes: Float32Array;
  readonly regWrites = new Float32Array(32);
  readonly activity = new Map<string, number>();
  disasm = new Map<number, string>();
  flash: Uint8Array | null = null;
  eeprom: Uint8Array | null = null;
  /** Bumped whenever the heat map changed (block fingerprint). */
  heatVersion = 0;
  private prevData: Uint8Array | null = null;
  private prevRegs: Uint8Array | null = null;
  private groupOf: Map<number, string>;

  constructor(readonly spec: AvrDeviceSpec) {
    this.heat = new Float32Array(spec.flashSize / 2);
    this.writes = new Float32Array(spec.sramStart + spec.sramSize);
    this.groupOf = new Map(spec.registers.map((r) => [r.addr, r.group]));
  }

  update(st: MachineState): void {
    // Execution heat: counts since the previous state (profiling), else the PC alone.
    const h = this.heat;
    let changed = false;
    if (st.execHeat && st.execHeat.length === h.length) {
      const c = st.execHeat;
      for (let i = 0; i < h.length; i++) {
        const add = c[i] ? Math.min(1, 0.3 + Math.log10(1 + c[i]) / 5) : 0;
        const v = Math.max(h[i] * HEAT_DECAY, add);
        if (v !== h[i]) {
          h[i] = v < 0.004 ? 0 : v;
          changed = true;
        }
      }
    } else if (st.pc < h.length) {
      for (let i = 0; i < h.length; i++) {
        if (h[i]) {
          h[i] = h[i] * HEAT_DECAY < 0.004 ? 0 : h[i] * HEAT_DECAY;
          changed = true;
        }
      }
      h[st.pc] = Math.max(h[st.pc], 0.6);
      changed = true;
    }
    if (changed) this.heatVersion++;

    // Data writes and peripheral activity.
    const w = this.writes;
    const d = st.data;
    const p = this.prevData;
    for (const [g, v] of this.activity) this.activity.set(g, v * WRITE_DECAY < 0.02 ? 0 : v * WRITE_DECAY);
    let io = false;
    for (let a = 0; a < w.length && a < d.length; a++) {
      if (p && p[a] !== d[a]) {
        w[a] = 1;
        if (a < this.spec.sramStart) {
          io = true;
          const g = this.groupOf.get(a);
          if (g) this.activity.set(g, 1);
        }
      } else if (w[a]) w[a] = w[a] * WRITE_DECAY < 0.03 ? 0 : w[a] * WRITE_DECAY;
    }
    if (io) this.activity.set('*io', 1);
    const rp = this.prevRegs;
    for (let r = 0; r < 32; r++) {
      if (rp && rp[r] !== st.regs[r]) this.regWrites[r] = 1;
      else if (this.regWrites[r]) this.regWrites[r] = this.regWrites[r] * WRITE_DECAY < 0.03 ? 0 : this.regWrites[r] * WRITE_DECAY;
    }
    this.prevData = d;
    this.prevRegs = st.regs;
  }

  data(st: MachineState, running: boolean): LiveData {
    return { spec: this.spec, st, running, heat: this.heat, writes: this.writes, regWrites: this.regWrites, activity: this.activity, disasm: this.disasm, flash: this.flash, eeprom: this.eeprom };
  }
}

export interface Layer {
  block: Block;
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  sig: string;
  /** Set when the canvas was redrawn (consumers clear it after uploading). */
  dirty: boolean;
}

/** One live canvas per block, `pxPerUm` resolution. */
export class BlockLayers {
  readonly layers: Layer[];

  constructor(plan: Floorplan, readonly pxPerUm: number) {
    this.layers = plan.blocks.map((block) => {
      const canvas = document.createElement('canvas');
      canvas.width = Math.max(8, Math.round(block.w * pxPerUm));
      canvas.height = Math.max(8, Math.round(block.h * pxPerUm));
      return { block, canvas, ctx: canvas.getContext('2d')!, sig: '\u0000', dirty: false };
    });
  }

  /** Redraws changed blocks; returns true when anything changed. */
  update(d: LiveData, heatVersion: number): boolean {
    let any = false;
    for (const l of this.layers) {
      const sig = blockSignature(l.block, d, heatVersion);
      if (sig === l.sig) continue;
      l.sig = sig;
      drawBlockLive(l.ctx, l.canvas.width, l.canvas.height, l.block, d);
      l.dirty = true;
      any = true;
    }
    return any;
  }
}

/** Resolution for block canvases / textures (capped for large dies). */
export function layerResolution(plan: Floorplan): number {
  return Math.min(1.25, 2200 / Math.max(plan.w, plan.h));
}

/** Static die artwork rendered once into a canvas at `pxPerUm`. */
export function renderDieBase(plan: Floorplan, spec: AvrDeviceSpec, pxPerUm: number): HTMLCanvasElement {
  const c = document.createElement('canvas');
  c.width = Math.round(plan.w * pxPerUm);
  c.height = Math.round(plan.h * pxPerUm);
  const ctx = c.getContext('2d')!;
  ctx.scale(pxPerUm, pxPerUm);
  drawDieBase(ctx, plan, spec);
  return c;
}

export interface Hover {
  x: number;
  y: number;
  hit: ReturnType<typeof hitTest>;
}

/** Flat die view on a 2D canvas: fit-to-window, wheel zoom, drag pan. */
export class Die2D {
  private ctx: CanvasRenderingContext2D;
  private base: HTMLCanvasElement;
  private zoom = 1;
  private panX = 0;
  private panY = 0;
  private raf = 0;
  private st: MachineState | null = null;
  private vcc = 5;
  /** Latest live data (for the zoomed-in byte view of the memory arrays). */
  private live: LiveData | null = null;
  /** Zoom at which the smallest memory cell is ~64 px wide (readable bytes and disassembly). */
  private maxZoom = 12;
  private ro: ResizeObserver;
  private cleanup: (() => void)[] = [];

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly plan: Floorplan,
    spec: AvrDeviceSpec,
    private readonly layers: BlockLayers,
    private readonly onHover: (h: Hover | null) => void,
    private readonly onClick: (h: Hover) => void,
  ) {
    this.ctx = canvas.getContext('2d')!;
    this.base = renderDieBase(plan, spec, Math.min(2, 3000 / Math.max(plan.w, plan.h)));
    this.ro = new ResizeObserver(() => this.request());
    this.ro.observe(canvas);
    const on = <K extends keyof HTMLElementEventMap>(t: K, f: (e: HTMLElementEventMap[K]) => void) => {
      canvas.addEventListener(t, f as EventListener, { passive: false });
      this.cleanup.push(() => canvas.removeEventListener(t, f as EventListener));
    };
    on('wheel', (e) => {
      e.preventDefault();
      const f = Math.exp(-e.deltaY * 0.0015);
      const r = canvas.getBoundingClientRect();
      const mx = e.clientX - r.left - r.width / 2;
      const my = e.clientY - r.top - r.height / 2;
      const nz = Math.max(0.5, Math.min(this.maxZoom, this.zoom * f));
      const k = nz / this.zoom;
      this.panX = mx - (mx - this.panX) * k;
      this.panY = my - (my - this.panY) * k;
      this.zoom = nz;
      this.request();
    });
    let drag: { x: number; y: number; px: number; py: number; moved: boolean } | null = null;
    on('pointerdown', (e) => {
      drag = { x: e.clientX, y: e.clientY, px: this.panX, py: this.panY, moved: false };
      canvas.setPointerCapture(e.pointerId);
    });
    on('pointermove', (e) => {
      if (drag) {
        const dx = e.clientX - drag.x;
        const dy = e.clientY - drag.y;
        if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
        this.panX = drag.px + dx;
        this.panY = drag.py + dy;
        this.request();
      }
      this.onHover(this.pick(e));
    });
    on('pointerup', (e) => {
      if (drag && !drag.moved) {
        const h = this.pick(e);
        if (h?.hit) this.onClick(h);
      }
      drag = null;
    });
    on('pointerleave', () => this.onHover(null));
    on('dblclick', () => {
      this.zoom = 1;
      this.panX = this.panY = 0;
      this.request();
    });
  }

  dispose(): void {
    cancelAnimationFrame(this.raf);
    this.ro.disconnect();
    for (const f of this.cleanup) f();
  }

  resetView(): void {
    this.zoom = 1;
    this.panX = this.panY = 0;
    this.request();
  }

  setState(st: MachineState, vcc: number, live?: LiveData): void {
    this.st = st;
    this.vcc = vcc;
    if (live) this.live = live;
    this.request();
  }

  /** Zooms onto a block (memory arrays: far enough to read the bytes). */
  focus(block: Block): void {
    const base = this.fit().s / this.zoom;
    const w = this.canvas.clientWidth;
    const h = this.canvas.clientHeight;
    let z = Math.min((w - 24) / block.w, (h - 52) / block.h) * 0.95 / base;
    const l = this.layers.layers.find((x) => x.block === block);
    const g = l && this.live ? memoryGrid(block, this.live.spec, l.canvas.width, l.canvas.height) : null;
    let cx = block.x + block.w / 2;
    let cy = block.y + block.h / 2;
    if (g && l) {
      // Readable cells: zoom to ~44 px per cell and start at the top-left of the array.
      const um = block.w / l.canvas.width;
      z = Math.max(z, Math.min(this.maxZoom, 44 / (g.cw * um * base)));
      const s = base * z;
      cx = Math.min(cx, block.x + (w / 2 - 12) / s);
      cy = Math.min(cy, block.y + g.top * um + (h / 2 - 40) / s);
    }
    this.zoom = Math.max(0.5, Math.min(this.maxZoom, z));
    const s = base * this.zoom;
    this.panX = s * (this.plan.w / 2 - cx);
    this.panY = s * (this.plan.h / 2 - cy);
    this.request();
  }

  /** Allows zooming in until the smallest memory cell is readable. */
  private updateMaxZoom(base: number): void {
    let z = 12;
    if (this.live) {
      for (const l of this.layers.layers) {
        const g = memoryGrid(l.block, this.live.spec, l.canvas.width, l.canvas.height);
        if (g) z = Math.max(z, 72 / (Math.min(g.cw, g.ch) * (l.block.w / l.canvas.width) * base));
      }
    }
    this.maxZoom = z;
  }

  request(): void {
    if (!this.raf) this.raf = requestAnimationFrame(() => {
      this.raf = 0;
      this.draw();
    });
  }

  /** CSS pixels -> die µm transform. */
  private fit(): { s: number; ox: number; oy: number; w: number; h: number } {
    const w = this.canvas.clientWidth;
    const h = this.canvas.clientHeight;
    // Leave room for the legend strip at the bottom.
    const s = Math.min((w - 24) / this.plan.w, (h - 52) / this.plan.h) * this.zoom;
    return { s, ox: w / 2 + this.panX - (this.plan.w * s) / 2, oy: (h - 28) / 2 + this.panY - (this.plan.h * s) / 2, w, h };
  }

  private pick(e: PointerEvent): Hover | null {
    const r = this.canvas.getBoundingClientRect();
    const f = this.fit();
    const x = (e.clientX - r.left - f.ox) / f.s;
    const y = (e.clientY - r.top - f.oy) / f.s;
    return { x: e.clientX - r.left, y: e.clientY - r.top, hit: hitTest(this.plan, x, y) };
  }

  private draw(): void {
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    const f = this.fit();
    if (f.w <= 0 || f.h <= 0) return;
    if (this.canvas.width !== Math.round(f.w * dpr) || this.canvas.height !== Math.round(f.h * dpr)) {
      this.canvas.width = Math.round(f.w * dpr);
      this.canvas.height = Math.round(f.h * dpr);
    }
    const ctx = this.ctx;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = '#1a2026';
    ctx.fillRect(0, 0, f.w, f.h);
    ctx.save();
    ctx.translate(f.ox, f.oy);
    ctx.scale(f.s, f.s);
    // Cheap drop shadow (a blur on the full die image would cost a few ms per frame).
    const sh = Math.max(this.plan.w, this.plan.h) * 0.012;
    ctx.fillStyle = 'rgba(0,0,0,0.35)';
    ctx.fillRect(sh, sh, this.plan.w, this.plan.h);
    ctx.fillStyle = 'rgba(0,0,0,0.25)';
    ctx.fillRect(sh * 0.5, sh * 0.5, this.plan.w, this.plan.h);
    ctx.drawImage(this.base, 0, 0, this.plan.w, this.plan.h);
    ctx.imageSmoothingQuality = 'high';
    this.updateMaxZoom(f.s / this.zoom);
    const detail: { l: Layer; g: MemGrid }[] = [];
    for (const l of this.layers.layers) {
      ctx.drawImage(l.canvas, l.block.x, l.block.y, l.block.w, l.block.h);
      const g = this.live && this.zoom > 1.5 ? memoryGrid(l.block, this.live.spec, l.canvas.width, l.canvas.height) : null;
      if (g) detail.push({ l, g });
    }
    for (const p of this.plan.pads) {
      ctx.fillStyle = pinColor(p, this.st, this.vcc);
      ctx.globalAlpha = 0.85;
      ctx.fillRect(p.x - p.s * 0.32, p.y - p.s * 0.32, p.s * 0.64, p.s * 0.64);
      ctx.globalAlpha = 1;
      ctx.fillStyle = '#0d1418';
      ctx.font = `700 ${p.s * 0.3}px Selawik, sans-serif`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(p.pin.name, p.x, p.y, p.s * 0.6);
      ctx.fillStyle = '#e6edf2';
      ctx.font = `${p.s * 0.34}px Selawik, sans-serif`;
      // Package pin number on the inner side of the pad (stays on the die).
      const dy = p.side === 'bottom' ? -p.s * 0.72 : p.side === 'top' ? p.s * 0.72 : 0;
      ctx.font = `600 ${p.s * 0.26}px Selawik, sans-serif`;
      ctx.fillText(`pin ${p.pin.number}`, p.x, p.y + dy);
      ctx.textAlign = 'left';
    }
    ctx.restore();
    // Memory arrays zoomed in far enough: individual bytes / words at screen resolution.
    for (const { l, g } of detail) {
      const k = (l.block.w / l.canvas.width) * f.s;
      drawMemoryDetail(ctx, l.block, this.live!, g, f.ox + l.block.x * f.s, f.oy + l.block.y * f.s, k, f);
    }
  }
}
