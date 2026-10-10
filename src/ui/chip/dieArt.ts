/**
 * Canvas drawing for the die: the static silicon artwork (memory arrays, standard-cell rows,
 * analog structures, pad ring, power rails, buses) and the live contents drawn over each block
 * (flash execution heat map + PC, SRAM bytes, registers, current instruction, SREG, peripheral
 * activity, pin levels). Used by the Device Info diagram (static) and the Chip View (2D and as
 * 3D textures). Pure 2D canvas code: no three.js here.
 */
import type { AvrDeviceSpec, MachineState } from '../backend/types';
import type { Block, BlockKind, Floorplan, Pad } from './floorplan';

const FONT = 'Selawik, "Segoe UI", sans-serif';
const MONO = '"Cascadia Mono", Consolas, monospace';

/** Deterministic pseudo random numbers (same artwork every time). */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const TINT: Record<BlockKind, [string, string]> = {
  flash: ['#4a3f74', '#7d6fb4'],
  eeprom: ['#5a3f6c', '#9a74b8'],
  sram: ['#28507a', '#5f93c8'],
  regs: ['#1f5f63', '#53a6a8'],
  decoder: ['#45573a', '#7f9a62'],
  alu: ['#5a4b30', '#b0915a'],
  control: ['#4c5634', '#8fa060'],
  iobus: ['#3f4d3c', '#7b9070'],
  periph: ['#3c5040', '#7fa07f'],
  analog: ['#6a4630', '#c08a5c'],
  clock: ['#5f3f3a', '#b97d6e'],
  nvm: ['#4b3d5a', '#8f78a8'],
};

const MEMORY: BlockKind[] = ['flash', 'eeprom', 'sram', 'regs'];

function memoryArray(ctx: CanvasRenderingContext2D, b: Block, light: string, pitch: number): void {
  const dec = b.w * 0.08;
  const amp = b.h * 0.1;
  ctx.fillStyle = 'rgba(0,0,0,0.18)';
  ctx.fillRect(b.x, b.y, dec, b.h - amp);
  ctx.fillRect(b.x, b.y + b.h - amp, b.w, amp);
  ctx.strokeStyle = light;
  ctx.globalAlpha = 0.35;
  ctx.lineWidth = pitch * 0.35;
  ctx.beginPath();
  for (let y = b.y + pitch; y < b.y + b.h - amp; y += pitch) {
    ctx.moveTo(b.x + dec, y);
    ctx.lineTo(b.x + b.w, y);
  }
  ctx.stroke();
  ctx.globalAlpha = 0.18;
  ctx.beginPath();
  for (let x = b.x + dec + pitch; x < b.x + b.w; x += pitch) {
    ctx.moveTo(x, b.y);
    ctx.lineTo(x, b.y + b.h - amp);
  }
  ctx.stroke();
  // Row decoder / sense amplifier detail.
  ctx.globalAlpha = 0.5;
  ctx.fillStyle = light;
  for (let y = b.y + pitch; y < b.y + b.h - amp; y += pitch * 2) ctx.fillRect(b.x + dec * 0.2, y, dec * 0.6, pitch * 0.5);
  for (let x = b.x + dec; x < b.x + b.w; x += pitch * 3) ctx.fillRect(x, b.y + b.h - amp * 0.8, pitch * 1.6, amp * 0.6);
  ctx.globalAlpha = 1;
}

function standardCells(ctx: CanvasRenderingContext2D, b: Block, light: string, rand: () => number): void {
  const rowH = Math.max(6, Math.min(b.w, b.h) / 14);
  for (let y = b.y + rowH * 0.3; y + rowH < b.y + b.h; y += rowH) {
    let x = b.x + 2;
    while (x < b.x + b.w - 2) {
      const w = Math.min(rowH * (0.3 + rand() * 1.6), b.x + b.w - 2 - x);
      ctx.globalAlpha = 0.18 + rand() * 0.35;
      ctx.fillStyle = rand() > 0.85 ? '#d8c27a' : light;
      ctx.fillRect(x, y + rowH * 0.12, w - rowH * 0.08, rowH * 0.76);
      x += w;
    }
  }
  // Vertical metal routing.
  ctx.globalAlpha = 0.25;
  ctx.strokeStyle = '#c9b46c';
  ctx.lineWidth = rowH * 0.12;
  ctx.beginPath();
  for (let i = 0; i < b.w / (rowH * 1.4); i++) {
    const x = b.x + rand() * b.w;
    const y0 = b.y + rand() * b.h * 0.5;
    ctx.moveTo(x, y0);
    ctx.lineTo(x, y0 + rand() * b.h * 0.6);
  }
  ctx.stroke();
  ctx.globalAlpha = 1;
}

function analogDevices(ctx: CanvasRenderingContext2D, b: Block, light: string, rand: () => number): void {
  const u = Math.min(b.w, b.h) / 10;
  // Capacitor array.
  ctx.fillStyle = light;
  for (let i = 0; i < 4; i++) {
    for (let j = 0; j < 3; j++) {
      ctx.globalAlpha = 0.25 + rand() * 0.25;
      ctx.fillRect(b.x + u * (0.6 + i * 1.3), b.y + u * (3 + j * 1.3), u, u);
    }
  }
  // Resistor serpentine.
  ctx.globalAlpha = 0.5;
  ctx.strokeStyle = '#e0b07a';
  ctx.lineWidth = u * 0.18;
  ctx.beginPath();
  let x = b.x + b.w * 0.62;
  ctx.moveTo(x, b.y + u * 3);
  for (let i = 0; i < 7; i++) {
    ctx.lineTo(x, b.y + b.h - u);
    x += u * 0.45;
    ctx.lineTo(x, b.y + b.h - u);
    ctx.lineTo(x, b.y + u * 3);
    x += u * 0.45;
    ctx.lineTo(x, b.y + u * 3);
  }
  ctx.stroke();
  // Guard ring.
  ctx.globalAlpha = 0.35;
  ctx.strokeStyle = light;
  ctx.lineWidth = u * 0.25;
  ctx.strokeRect(b.x + u * 0.3, b.y + u * 2.6, b.w - u * 0.6, b.h - u * 2.9);
  ctx.globalAlpha = 1;
}

/** Static die artwork in µm coordinates (caller sets the transform). */
export function drawDieBase(ctx: CanvasRenderingContext2D, plan: Floorplan, spec: AvrDeviceSpec): void {
  const { w: W, h: H } = plan;
  const rand = rng(0xa7);
  const g = ctx.createLinearGradient(0, 0, W, H);
  g.addColorStop(0, '#3d4c50');
  g.addColorStop(0.5, '#33403f');
  g.addColorStop(1, '#2a3437');
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, W, H);
  // Scribe line and seal ring.
  const e = Math.min(W, H) * 0.012;
  ctx.strokeStyle = '#9aa6aa';
  ctx.lineWidth = e;
  ctx.strokeRect(e, e, W - 2 * e, H - 2 * e);
  ctx.strokeStyle = '#6d7a7e';
  ctx.lineWidth = e * 0.4;
  ctx.strokeRect(e * 2.4, e * 2.4, W - 4.8 * e, H - 4.8 * e);
  // Power rails around the core.
  const c = plan.core;
  for (const [off, col] of [[e * 1.6, '#b39b5c'], [e * 3.4, '#8f8a8a']] as [number, string][]) {
    ctx.strokeStyle = col;
    ctx.lineWidth = e * 1.1;
    ctx.strokeRect(c.x - off, c.y - off, c.w + 2 * off, c.h + 2 * off);
  }
  // Blocks.
  for (const b of plan.blocks) {
    const [base, light] = TINT[b.kind];
    ctx.fillStyle = base;
    ctx.fillRect(b.x, b.y, b.w, b.h);
    if (MEMORY.includes(b.kind)) memoryArray(ctx, b, light, b.kind === 'flash' ? Math.max(3, b.h / 60) : Math.max(4, b.h / 26));
    else if (b.kind === 'analog' || b.kind === 'clock') analogDevices(ctx, b, light, rand);
    else standardCells(ctx, b, light, rand);
    ctx.strokeStyle = 'rgba(210,220,225,0.35)';
    ctx.lineWidth = e * 0.25;
    ctx.strokeRect(b.x, b.y, b.w, b.h);
  }
  // Buses.
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  for (const bus of plan.buses) {
    ctx.strokeStyle = 'rgba(201,180,108,0.55)';
    ctx.lineWidth = e * (bus.id === 'pbus' ? 1.3 : 0.9);
    ctx.beginPath();
    bus.points.forEach(([x, y], i) => (i ? ctx.lineTo(x, y) : ctx.moveTo(x, y)));
    ctx.stroke();
  }
  // Pads, ESD structures and pad-to-core routing.
  for (const p of plan.pads) {
    const toward = p.side === 'bottom' ? -1 : p.side === 'top' ? 1 : 0;
    const towardX = p.side === 'left' ? 1 : p.side === 'right' ? -1 : 0;
    ctx.strokeStyle = 'rgba(201,180,108,0.6)';
    ctx.lineWidth = p.s * 0.18;
    ctx.beginPath();
    ctx.moveTo(p.x, p.y);
    ctx.lineTo(p.x + towardX * p.s * 1.6, p.y + toward * p.s * 1.6);
    ctx.stroke();
    ctx.fillStyle = '#6e5a3a';
    ctx.fillRect(p.x - p.s * 0.45 + towardX * p.s * 0.9, p.y - p.s * 0.45 + toward * p.s * 0.9, p.s * 0.9, p.s * 0.5);
    ctx.fillStyle = '#c9ced6';
    ctx.fillRect(p.x - p.s / 2, p.y - p.s / 2, p.s, p.s);
    ctx.strokeStyle = '#7f8790';
    ctx.lineWidth = p.s * 0.08;
    ctx.strokeRect(p.x - p.s / 2, p.y - p.s / 2, p.s, p.s);
  }
  // Die marking.
  ctx.fillStyle = 'rgba(220,226,230,0.45)';
  ctx.font = `600 ${Math.round(e * 2.2)}px ${FONT}`;
  ctx.textBaseline = 'top';
  ctx.fillText(`MCS ${spec.name} model`, c.x, e * 4.2);
  ctx.textAlign = 'right';
  ctx.fillText(spec.coreName, c.x + c.w, e * 4.2);
  ctx.textAlign = 'left';
}

/** Block names and descriptions (static diagram). */
export function drawDieLabels(ctx: CanvasRenderingContext2D, plan: Floorplan): void {
  for (const b of plan.blocks) {
    const fs = Math.max(9, Math.min(b.h * 0.16, b.w * 0.09, 34));
    ctx.fillStyle = 'rgba(10,16,20,0.55)';
    ctx.fillRect(b.x, b.y, b.w, fs * 2.3);
    ctx.fillStyle = '#f2f6f8';
    ctx.font = `700 ${fs}px ${FONT}`;
    ctx.textBaseline = 'top';
    ctx.fillText(b.label, b.x + fs * 0.4, b.y + fs * 0.2, b.w - fs * 0.8);
    ctx.fillStyle = '#c4d0d6';
    ctx.font = `${fs * 0.72}px ${FONT}`;
    ctx.fillText(b.sub, b.x + fs * 0.4, b.y + fs * 1.3, b.w - fs * 0.8);
  }
  for (const p of plan.pads) {
    const fs = p.s * 0.42;
    ctx.fillStyle = '#ffffff';
    ctx.font = `700 ${fs}px ${FONT}`;
    ctx.textAlign = 'center';
    ctx.textBaseline = p.side === 'bottom' ? 'top' : 'bottom';
    const dy = p.side === 'bottom' ? p.s * 0.6 : -p.s * 0.6;
    ctx.fillText(`${p.pin.number}`, p.x, p.y + dy);
    ctx.textBaseline = 'middle';
    ctx.fillStyle = '#1b2a33';
    ctx.font = `700 ${fs * 0.9}px ${FONT}`;
    ctx.fillText(p.pin.name, p.x, p.y, p.s * 0.95);
    ctx.textAlign = 'left';
  }
}

// ------------------------------------------------------------------------------------ live

/** Per-frame inputs for the live overlays. */
export interface LiveData {
  spec: AvrDeviceSpec;
  st: MachineState;
  running: boolean;
  /** Decayed execution heat per flash word (0..1). */
  heat: Float32Array;
  /** Decayed write highlight per data address (0..1). */
  writes: Float32Array;
  /** Decayed change highlight per CPU register (0..1). */
  regWrites: Float32Array;
  /** Program memory (latest image). */
  flash: Uint8Array | null;
  /** EEPROM contents (latest image). */
  eeprom: Uint8Array | null;
  /** Flash words with non-zero heat. */
  hot: number[];
  /** Decayed activity per register group (0..1). */
  activity: Map<string, number>;
  /** Disassembly text by word address. */
  disasm: Map<number, string>;
}

const hex2 = (v: number) => v.toString(16).toUpperCase().padStart(2, '0');
const hex4 = (v: number) => v.toString(16).toUpperCase().padStart(4, '0');

function heatColor(t: number, programmed: boolean): string {
  if (t <= 0.004) return programmed ? '#5b4f8c' : '#2c2742';
  // purple -> orange -> yellow -> white
  const r = Math.round(120 + 135 * Math.min(1, t * 1.6));
  const g = Math.round(80 + 175 * Math.max(0, t * 1.4 - 0.25));
  const b = Math.round(140 * (1 - Math.min(1, t * 1.8)) + 120 * Math.max(0, t - 0.75) * 4);
  return `rgb(${r},${Math.min(255, g)},${Math.min(255, b)})`;
}

function header(ctx: CanvasRenderingContext2D, w: number, label: string, sub: string, fs: number, glow = 0): number {
  ctx.fillStyle = glow > 0.02 ? `rgba(${Math.round(40 + 120 * glow)},${Math.round(70 + 110 * glow)},40,0.78)` : 'rgba(8,14,18,0.62)';
  ctx.fillRect(0, 0, w, fs * 1.55);
  ctx.fillStyle = '#ffffff';
  ctx.font = `700 ${fs}px ${FONT}`;
  ctx.textBaseline = 'middle';
  ctx.fillText(label, fs * 0.4, fs * 0.8, w * 0.55);
  ctx.fillStyle = '#b9c8cf';
  ctx.font = `${fs * 0.72}px ${FONT}`;
  ctx.textAlign = 'right';
  ctx.fillText(sub, w - fs * 0.4, fs * 0.82, w * 0.45);
  ctx.textAlign = 'left';
  return fs * 1.55;
}

function cellGrid(x0: number, y0: number, w: number, h: number, n: number, cols: number, draw: (i: number, x: number, y: number, cw: number, ch: number) => void): void {
  const rows = Math.ceil(n / cols);
  const cw = w / cols;
  const ch = h / rows;
  for (let i = 0; i < n; i++) draw(i, x0 + (i % cols) * cw, y0 + Math.floor(i / cols) * ch, cw, ch);
}

/** Header font size of a block canvas of `w x h` pixels. */
function headerFs(w: number, h: number): number {
  return Math.max(10, Math.min(h * 0.11, w * 0.06, 30));
}

/** Cell layout of a large memory array (flash words, SRAM / EEPROM bytes) in a `w x h` canvas. */
export interface MemGrid {
  /** Cells (flash: words, others: bytes). */
  n: number;
  cols: number;
  /** First address shown and address step per cell (flash: byte address of each word). */
  base: number;
  step: number;
  top: number;
  cw: number;
  ch: number;
}

/** Number of cells of a memory block (0 when the block has no array). */
function memCells(b: Block, spec: AvrDeviceSpec): number {
  return b.kind === 'flash' ? spec.flashSize / 2 : b.kind === 'sram' ? spec.sramSize : b.kind === 'eeprom' ? spec.eepromSize : 0;
}

/** True for memory blocks drawn as a plain cell array (small SRAMs get the labelled hex grid). */
function isArray(b: Block, spec: AvrDeviceSpec): boolean {
  const n = memCells(b, spec);
  return n > 0 && (b.kind === 'flash' || n > 128);
}

/** Layout shared by the block texture and the zoomed byte view, so both line up exactly. */
export function memoryGrid(b: Block, spec: AvrDeviceSpec, w: number, h: number): MemGrid | null {
  if (!isArray(b, spec)) return null;
  const n = memCells(b, spec);
  const top = headerFs(w, h) * 1.55;
  const cols = Math.max(1, Math.round(Math.sqrt(n * (w / (h - top)))));
  const rows = Math.ceil(n / cols);
  return { n, cols, base: b.kind === 'sram' ? spec.sramStart : 0, step: b.kind === 'flash' ? 2 : 1, top, cw: w / cols, ch: (h - top) / rows };
}

/** One pixel per flash word (programmed / erased), rebuilt only when the image or grid changes. */
const flashImages = new WeakMap<Uint8Array, { cols: number; c: HTMLCanvasElement }>();
const ERASED_FLASH = new Uint8Array(0);
function flashImage(flash: Uint8Array | null, g: MemGrid): HTMLCanvasElement {
  const key = flash ?? ERASED_FLASH;
  const hit = flashImages.get(key);
  if (hit && hit.cols === g.cols) return hit.c;
  const rows = Math.ceil(g.n / g.cols);
  const c = document.createElement('canvas');
  c.width = g.cols;
  c.height = rows;
  const ctx = c.getContext('2d')!;
  const img = ctx.createImageData(g.cols, rows);
  const px = new Uint32Array(img.data.buffer);
  // ABGR (little-endian): programmed #5b4f8c, erased #2c2742.
  const on = 0xff8c4f5b;
  const off = 0xff42272c;
  for (let i = 0; i < g.n; i++) px[i] = !flash || flash[i * 2] !== 0xff || flash[i * 2 + 1] !== 0xff ? on : off;
  ctx.putImageData(img, 0, 0);
  flashImages.set(key, { cols: g.cols, c });
  return c;
}

/** One pixel per SRAM / EEPROM byte (same colours as `memCell`), in a reused scratch canvas. */
const byteCanvases = new Map<string, { c: HTMLCanvasElement; img: ImageData }>();
function byteImage(b: Block, d: LiveData, g: MemGrid): HTMLCanvasElement {
  const rows = Math.ceil(g.n / g.cols);
  let e = byteCanvases.get(b.id);
  if (!e || e.img.width !== g.cols || e.img.height !== rows) {
    const c = document.createElement('canvas');
    c.width = g.cols;
    c.height = rows;
    e = { c, img: c.getContext('2d')!.createImageData(g.cols, rows) };
    byteCanvases.set(b.id, e);
  }
  const px = new Uint32Array(e.img.data.buffer);
  const eep = b.kind === 'eeprom';
  const { st, writes } = d;
  const base = d.spec.sramStart;
  for (let i = 0; i < g.n; i++) {
    const a = base + i;
    const v = eep ? d.eeprom?.[i] ?? 0xff : st.data[a] ?? 0;
    const wr = eep ? 0 : writes[a] ?? 0;
    let r: number, gg: number, bb: number;
    if (wr > 0.03) [r, gg, bb] = [255, 170 - 40 * wr, 40];
    else if (eep) [r, gg, bb] = [40 + v / 4, 40 + v / 3, 70 + v / 3];
    else if (a > st.sp) [r, gg, bb] = [30 + v / 4, 70 + v / 3, 120 + v / 3];
    else [r, gg, bb] = [30 + v / 3, 50 + v / 2.2, 80 + v / 2];
    px[i] = 0xff000000 | (bb << 16) | (gg << 8) | r;
  }
  e.c.getContext('2d')!.putImageData(e.img, 0, 0);
  return e.c;
}

/** Cell value and fill colour of memory cell `i` (array view). */
function memCell(b: Block, d: LiveData, i: number): { v: number; fill: string } {
  if (b.kind === 'flash') {
    const f = d.flash;
    const v = f ? f[i * 2] | (f[i * 2 + 1] << 8) : 0xffff;
    return { v, fill: heatColor(d.heat[i] ?? 0, !f || v !== 0xffff) };
  }
  if (b.kind === 'eeprom') {
    const v = d.eeprom?.[i] ?? 0xff;
    return { v, fill: `rgb(${40 + v / 4},${40 + v / 3},${70 + v / 3})` };
  }
  const a = d.spec.sramStart + i;
  const v = d.st.data[a] ?? 0;
  const wr = d.writes[a] ?? 0;
  const fill = wr > 0.03 ? `rgba(255,170,40,${0.4 + 0.6 * wr})` : a > d.st.sp ? `rgb(${30 + v / 4},${70 + v / 3},${120 + v / 3})` : `rgb(${30 + v / 3},${50 + v / 2.2},${80 + v / 2})`;
  return { v, fill };
}

/**
 * Zoomed-in byte view of a memory block, drawn in screen space at screen resolution: only the
 * cells inside `view` (screen pixels) are visited, so cost depends on the window, not the
 * memory size. `ox/oy/s` map block-canvas pixels (`cw x ch` grid of `g`) to the screen.
 * Returns false when the cells are still too small for text (the texture is shown instead).
 */
export function drawMemoryDetail(
  ctx: CanvasRenderingContext2D, b: Block, d: LiveData, g: MemGrid, ox: number, oy: number, s: number, view: { w: number; h: number },
): boolean {
  const cw = g.cw * s;
  const ch = g.ch * s;
  const flash = b.kind === 'flash';
  if (cw < (flash ? 30 : 17) || ch < 11) return false;
  const rows = Math.ceil(g.n / g.cols);
  const y0 = oy + g.top * s;
  const c0 = Math.max(0, Math.floor(-ox / cw));
  const c1 = Math.min(g.cols - 1, Math.floor((view.w - ox) / cw));
  const r0 = Math.max(0, Math.floor(-(y0) / ch));
  const r1 = Math.min(rows - 1, Math.floor((view.h - y0) / ch));
  if (c0 > c1 || r0 > r1) return true;
  const pad = Math.max(1, cw * 0.04);
  const fsV = Math.min(ch * 0.42, cw / (flash ? 2.9 : 1.6));
  const showAddr = ch >= 30 && cw >= 34;
  const showAsm = flash && ch >= 44 && cw >= 70;
  const fsA = Math.min(ch * 0.2, cw / 6.5);
  const pc = d.st.pc;
  const sp = d.st.sp;
  ctx.save();
  ctx.beginPath();
  ctx.rect(ox, y0, g.cols * cw, rows * ch);
  ctx.clip();
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  for (let r = r0; r <= r1; r++) {
    for (let c = c0; c <= c1; c++) {
      const i = r * g.cols + c;
      if (i >= g.n) break;
      const x = ox + c * cw;
      const y = y0 + r * ch;
      const cell = memCell(b, d, i);
      ctx.fillStyle = 'rgba(6,10,14,0.9)';
      ctx.fillRect(x, y, cw, ch);
      ctx.fillStyle = cell.fill;
      ctx.fillRect(x + pad, y + pad, cw - 2 * pad, ch - 2 * pad);
      const addr = g.base + i * g.step;
      if ((flash && i === pc) || (b.kind === 'sram' && addr === sp)) {
        ctx.strokeStyle = flash ? '#ffffff' : '#7ff0ff';
        ctx.lineWidth = Math.max(2, cw * 0.05);
        ctx.strokeRect(x + pad, y + pad, cw - 2 * pad, ch - 2 * pad);
      }
      const ty = showAsm ? y + ch * 0.42 : showAddr ? y + ch * 0.58 : y + ch / 2;
      ctx.fillStyle = '#ffffff';
      ctx.font = `600 ${fsV}px ${MONO}`;
      ctx.fillText(flash ? hex4(cell.v) : hex2(cell.v), x + cw / 2, ty, cw - 2 * pad);
      if (showAddr) {
        ctx.fillStyle = 'rgba(220,232,240,0.75)';
        ctx.font = `${fsA}px ${MONO}`;
        ctx.textAlign = 'left';
        ctx.fillText(hex4(addr), x + pad * 2, y + pad + fsA * 0.75, cw - 4 * pad);
        ctx.textAlign = 'center';
      }
      if (showAsm) {
        const t = d.disasm.get(i);
        if (t) {
          ctx.fillStyle = '#ffe9a8';
          ctx.font = `${Math.min(ch * 0.2, cw / 9)}px ${MONO}`;
          ctx.fillText(t, x + cw / 2, y + ch * 0.74, cw - 3 * pad);
        }
      }
    }
  }
  ctx.restore();
  return true;
}

/**
 * Draws one block's live overlay into a canvas of `w x h` pixels (the block's rectangle).
 * The background stays translucent so the silicon artwork shows through.
 */
export function drawBlockLive(ctx: CanvasRenderingContext2D, w: number, h: number, b: Block, d: LiveData): void {
  ctx.clearRect(0, 0, w, h);
  const fs = headerFs(w, h);
  const st = d.st;
  const spec = d.spec;
  switch (b.kind) {
    case 'flash': {
      header(ctx, w, b.label, `${spec.flashSize} B - PC 0x${hex4(st.pc * 2)}`, fs);
      const g = memoryGrid(b, spec, w, h)!;
      const { cols, top, cw, ch } = g;
      const pad = Math.max(1, cw * 0.08);
      if (cw >= 2 && ch >= 2) {
        cellGrid(0, top, w, h - top, g.n, cols, (i, x, y, cw, ch) => {
          ctx.fillStyle = memCell(b, d, i).fill;
          ctx.fillRect(x + pad / 2, y + pad / 2, cw - pad, ch - pad);
        });
      } else {
        // Sub-pixel cells (large flash): cached programmed/erased image + the warm words only.
        ctx.imageSmoothingEnabled = false;
        ctx.drawImage(flashImage(d.flash, g), 0, top, cols * cw, Math.ceil(g.n / cols) * ch);
        ctx.imageSmoothingEnabled = true;
        const mw = Math.max(1, cw);
        const mh = Math.max(1, ch);
        for (const i of d.hot) {
          ctx.fillStyle = heatColor(d.heat[i], true);
          ctx.fillRect((i % cols) * cw, top + Math.floor(i / cols) * ch, mw, mh);
        }
      }
      // PC marker.
      const px = (st.pc % cols) * cw;
      const py = top + Math.floor(st.pc / cols) * ch;
      ctx.strokeStyle = '#ffffff';
      ctx.lineWidth = Math.max(2, cw * 0.18);
      ctx.strokeRect(px, py, cw, ch);
      ctx.strokeStyle = 'rgba(120,220,255,0.9)';
      ctx.lineWidth = Math.max(1, cw * 0.08);
      ctx.strokeRect(px - cw * 0.25, py - ch * 0.25, cw * 1.5, ch * 1.5);
      return;
    }
    case 'sram':
    case 'eeprom': {
      const base = b.kind === 'sram' ? spec.sramStart : 0;
      const n = memCells(b, spec);
      const top = header(ctx, w, b.label, b.kind === 'sram' ? `SP 0x${hex4(st.sp)}` : b.sub, fs);
      if (!n) return;
      const g = memoryGrid(b, spec, w, h);
      if (!g) {
        const cols = n <= 32 ? 8 : 16;
        const addrW = fs * 3.3;
        cellGrid(addrW, top, w - addrW, h - top, n, cols, (i, x, y, cw, ch) => {
          const a = base + i;
          const eep = b.kind === 'eeprom';
          const v = eep ? d.eeprom?.[i] ?? 0xff : st.data[a] ?? 0;
          const wr = eep ? 0 : d.writes[a] ?? 0;
          const stack = !eep && a > st.sp;
          ctx.fillStyle = wr > 0.03 ? `rgba(255,${Math.round(150 + 60 * (1 - wr))},40,${0.35 + 0.55 * wr})` : stack ? 'rgba(80,160,230,0.32)' : 'rgba(10,20,30,0.42)';
          ctx.fillRect(x + 1, y + 1, cw - 2, ch - 2);
          if (!eep && a === st.sp) {
            ctx.strokeStyle = '#7ff0ff';
            ctx.lineWidth = 2;
            ctx.strokeRect(x + 1, y + 1, cw - 2, ch - 2);
          }
          ctx.fillStyle = '#ffffff';
          ctx.font = `${Math.min(ch * 0.5, cw * 0.42)}px ${MONO}`;
          ctx.textAlign = 'center';
          ctx.textBaseline = 'middle';
          ctx.fillText(hex2(v), x + cw / 2, y + ch / 2);
          ctx.textAlign = 'left';
          if (i % cols === 0) {
            ctx.fillStyle = '#b9c8cf';
            ctx.font = `${Math.min(ch * 0.36, fs * 0.8)}px ${MONO}`;
            ctx.fillText(hex4(a), 2, y + ch / 2);
          }
        });
      } else {
        if (g.cw >= 2 && g.ch >= 2) {
          cellGrid(0, g.top, w, h - g.top, n, g.cols, (i, x, y, cw, ch) => {
            ctx.fillStyle = memCell(b, d, i).fill;
            ctx.fillRect(x, y, cw, ch);
          });
        } else {
          ctx.imageSmoothingEnabled = false;
          ctx.drawImage(byteImage(b, d, g), 0, g.top, g.cols * g.cw, Math.ceil(n / g.cols) * g.ch);
          ctx.imageSmoothingEnabled = true;
        }
      }
      return;
    }
    case 'regs': {
      const first = spec.features & 1 ? 16 : 0;
      const n = 32 - first;
      const top = header(ctx, w, b.label, b.sub, fs);
      cellGrid(0, top, w, h - top, n, n === 16 ? 4 : 8, (i, x, y, cw, ch) => {
        const r = first + i;
        const v = st.regs[r];
        const wr = d.regWrites[r] ?? 0;
        ctx.fillStyle = wr > 0.03 ? `rgba(255,170,40,${0.35 + 0.55 * wr})` : 'rgba(10,30,32,0.45)';
        ctx.fillRect(x + 1, y + 1, cw - 2, ch - 2);
        ctx.fillStyle = '#9fd6d6';
        ctx.font = `${Math.min(ch * 0.28, cw * 0.2)}px ${FONT}`;
        ctx.textBaseline = 'top';
        ctx.fillText(`R${r}`, x + cw * 0.08, y + ch * 0.06);
        ctx.fillStyle = '#ffffff';
        ctx.font = `${Math.min(ch * 0.46, cw * 0.32)}px ${MONO}`;
        ctx.textAlign = 'right';
        ctx.textBaseline = 'bottom';
        ctx.fillText(hex2(v), x + cw * 0.92, y + ch * 0.94);
        ctx.textAlign = 'left';
      });
      return;
    }
    case 'decoder': {
      const top = header(ctx, w, b.label, `PC 0x${hex4(st.pc * 2)}`, fs);
      const flash = d.flash;
      const word = flash ? flash[st.pc * 2] | (flash[st.pc * 2 + 1] << 8) : 0;
      const text = d.disasm.get(st.pc) ?? '';
      ctx.fillStyle = '#ffe9a8';
      ctx.font = `700 ${Math.min(fs * 1.5, (h - top) * 0.32)}px ${MONO}`;
      ctx.textBaseline = 'middle';
      ctx.fillText(text || '...', fs * 0.5, top + (h - top) * 0.36, w - fs);
      ctx.fillStyle = '#c4d0d6';
      ctx.font = `${Math.min(fs * 1.05, (h - top) * 0.22)}px ${MONO}`;
      ctx.fillText(`${hex4(word)}  ${word.toString(2).padStart(16, '0').replace(/(.{4})(?!$)/g, '$1 ')}`, fs * 0.5, top + (h - top) * 0.75, w - fs);
      return;
    }
    case 'alu': {
      const top = header(ctx, w, b.label, 'SREG', fs);
      const names = 'ITHSVNZC';
      cellGrid(fs * 0.3, top + fs * 0.3, w - fs * 0.6, h - top - fs * 0.6, 8, 4, (i, x, y, cw, ch) => {
        const on = (st.sreg >> (7 - i)) & 1;
        ctx.fillStyle = on ? '#3fd06a' : 'rgba(10,20,15,0.55)';
        ctx.fillRect(x + 2, y + 2, cw - 4, ch - 4);
        ctx.fillStyle = on ? '#0b2a12' : '#9fb7a5';
        ctx.font = `700 ${Math.min(ch * 0.55, cw * 0.55)}px ${MONO}`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText(names[i], x + cw / 2, y + ch / 2);
        ctx.textAlign = 'left';
      });
      return;
    }
    case 'control': {
      const status = st.resetHeld ? 'RESET' : st.sleeping ? 'SLEEP' : d.running ? 'RUN' : 'HALT';
      const top = header(ctx, w, b.label, status, fs, d.running && !st.sleeping ? 0.6 : 0);
      const lines = [`PC   0x${hex4(st.pc * 2)}`, `SP   0x${hex4(st.sp)}`, `CYC  ${st.cycles.toLocaleString()}`];
      ctx.fillStyle = '#ffffff';
      const lh = (h - top) / 3.3;
      ctx.font = `${Math.min(lh * 0.62, fs * 1.1)}px ${MONO}`;
      ctx.textBaseline = 'middle';
      lines.forEach((l, i) => ctx.fillText(l, fs * 0.5, top + lh * (i + 0.65), w - fs));
      return;
    }
    case 'iobus': {
      const irqDepth = st.callStack.filter((f) => f.vector >= 0).length;
      const top = header(ctx, w, 'I/O & IRQ', `I=${(st.sreg >> 7) & 1}`, fs, d.activity.get('*io') ?? 0);
      const lh = (h - top) / 3;
      ctx.fillStyle = '#ffffff';
      ctx.font = `${Math.min(lh * 0.55, fs)}px ${FONT}`;
      ctx.textBaseline = 'middle';
      const frames = st.callStack;
      const lastIrq = [...frames].reverse().find((f) => f.vector >= 0);
      ctx.fillText(irqDepth ? `In ISR: ${spec.vectors.find((v) => v.index === lastIrq?.vector)?.name ?? lastIrq?.vector}` : 'No interrupt active', fs * 0.5, top + lh * 0.6, w - fs);
      ctx.fillStyle = '#c4d0d6';
      ctx.fillText(`Call depth ${frames.length}`, fs * 0.5, top + lh * 1.6, w - fs);
      return;
    }
    default: {
      // Peripheral / clock / NVM blocks: activity glow + key values.
      const act = d.activity.get(b.group ?? '') ?? 0;
      const lines = peripheralLines(b, d);
      if (act > 0.02) {
        ctx.fillStyle = `rgba(90,255,140,${0.18 * act})`;
        ctx.fillRect(0, 0, w, h);
        ctx.strokeStyle = `rgba(120,255,160,${0.85 * act})`;
        ctx.lineWidth = Math.max(2, fs * 0.25);
        ctx.strokeRect(1, 1, w - 2, h - 2);
      }
      const top = header(ctx, w, b.label, '', fs, act);
      // Port blocks show their pins as LEDs.
      const portPins = /^PORT([A-Z])$/.exec(b.group ?? '');
      if (portPins) {
        const pins = spec.pins.filter((p) => p.gpio !== undefined && p.name.startsWith(`P${portPins[1]}`));
        cellGrid(fs * 0.3, top + fs * 0.2, w - fs * 0.6, h - top - fs * 0.4, pins.length, Math.min(pins.length, 4), (i, x, y, cw, ch) => {
          const ps = st.pins[pins[i].gpio!];
          const r = Math.min(cw, ch) * 0.26;
          ctx.beginPath();
          ctx.arc(x + cw / 2, y + ch * 0.4, r, 0, Math.PI * 2);
          ctx.fillStyle = ps?.level ? (ps.dir ? '#5dff7a' : '#59b8ff') : '#203028';
          ctx.fill();
          ctx.fillStyle = '#ffffff';
          ctx.font = `${Math.min(ch * 0.26, cw * 0.28)}px ${FONT}`;
          ctx.textAlign = 'center';
          ctx.textBaseline = 'top';
          ctx.fillText(`${pins[i].name}${ps?.dir ? ' out' : ''}`, x + cw / 2, y + ch * 0.72);
          ctx.textAlign = 'left';
        });
        return;
      }
      const lh = (h - top) / Math.max(3, lines.length + 0.5);
      ctx.fillStyle = '#ffffff';
      ctx.font = `${Math.min(lh * 0.62, fs * 0.9)}px ${FONT}`;
      ctx.textBaseline = 'middle';
      lines.forEach((l, i) => ctx.fillText(l, fs * 0.4, top + lh * (i + 0.7), w - fs * 0.8));
    }
  }
}

/** Key values shown in a peripheral / clock / NVM block. */
function peripheralLines(b: Block, d: LiveData): string[] {
  const { st, spec } = d;
  const info = st.peripherals.find((p) => p.name === (b.kind === 'clock' ? 'SYSTEM' : b.group));
  if (b.kind === 'clock') return [st.hz >= 1e6 ? `${+(st.hz / 1e6).toFixed(3)} MHz` : `${+(st.hz / 1e3).toFixed(1)} kHz`, info?.values.find((v) => v[0] === 'Clock source')?.[1] ?? ''];
  if (b.kind === 'nvm') return [`Fuses ${st.fuses.map(hex2).join(' ')}  Lock ${hex2(st.lock)}`, `Sig ${spec.signature.map(hex2).join(' ')}`];
  if (info) return info.values.slice(0, 3).map(([k, v]) => `${k}: ${v}`);
  return spec.registers.filter((r) => r.group === b.group).slice(0, 3).map((r) => `${r.name} 0x${hex2(st.data[r.addr] ?? 0)}`);
}

const q = (v: number | undefined) => Math.round((v ?? 0) * 10);

/**
 * Cheap fingerprint of everything `drawBlockLive` shows for a block: unchanged fingerprints
 * skip both the redraw and the texture upload.
 */
export function blockSignature(b: Block, d: LiveData, heatVersion: number): string {
  const { st, spec } = d;
  switch (b.kind) {
    case 'flash':
      return `${st.pc}|${heatVersion}`;
    case 'sram': {
      // Numeric hash: SRAM can be tens of KB (a string per byte would be far slower).
      let hsh = st.sp;
      for (let a = spec.sramStart; a < spec.sramStart + spec.sramSize; a++) hsh = (Math.imul(hsh, 31) + st.data[a] * 16 + q(d.writes[a])) | 0;
      return `${hsh}`;
    }
    case 'eeprom': {
      // Cheap content hash; the EEPROM image changes rarely.
      const e = d.eeprom;
      let hsh = 0;
      if (e) for (let i = 0; i < e.length; i++) hsh = (Math.imul(hsh, 31) + e[i]) | 0;
      return `${hsh}`;
    }
    case 'regs': {
      let s = '';
      for (let r = 0; r < 32; r++) s += `${st.regs[r]},${q(d.regWrites[r])};`;
      return s;
    }
    case 'decoder':
      return `${st.pc}|${d.disasm.get(st.pc) ?? ''}|${d.flash ? d.flash[st.pc * 2] | (d.flash[st.pc * 2 + 1] << 8) : 0}`;
    case 'alu':
      return `${st.sreg}`;
    case 'control':
      return `${st.pc}|${st.sp}|${st.cycles}|${st.sleeping}|${st.resetHeld}|${d.running}`;
    case 'iobus':
      return `${st.sreg >> 7}|${st.callStack.length}|${st.callStack[st.callStack.length - 1]?.vector}|${q(d.activity.get('*io'))}`;
    default: {
      const pins = /^PORT/.test(b.group ?? '') ? st.pins.map((p) => `${p.level}${p.dir}`).join('') : '';
      return `${q(d.activity.get(b.group ?? ''))}|${pins}|${peripheralLines(b, d).join('|')}`;
    }
  }
}

/** Colour of a bond pad / wire / lead for a pin state. */
export function pinColor(pad: Pad, st: MachineState | null, vcc: number): string {
  if (pad.pin.kind === 'vcc') return '#ff6a4a';
  if (pad.pin.kind === 'gnd') return '#55606a';
  const p = st && pad.pin.gpio !== undefined ? st.pins[pad.pin.gpio] : undefined;
  if (!p) return '#9aa3ab';
  if (!p.dir && p.ext === 'analog') {
    const t = Math.min(1, p.volts / Math.max(0.1, vcc));
    return `rgb(${Math.round(150 + 105 * t)},${Math.round(110 + 80 * t)},40)`;
  }
  if (p.level) return p.dir ? '#4dff6e' : '#4fb2ff';
  return p.dir ? '#1f6b2c' : '#7d8790';
}
