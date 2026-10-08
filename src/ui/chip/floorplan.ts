/**
 * Illustrative die floorplan generated from a device spec: memory arrays, CPU core, one block per
 * peripheral group, oscillators/NVM, the bond-pad ring and the main buses. Coordinates are in
 * micrometres on the die (x right, y down = towards package pin 1's row). The real die size comes
 * from the spec when known; block placement is a plausible arrangement, not a traced layout.
 */
import type { AvrDeviceSpec, PinSpec } from '../backend/types';

export type BlockKind = 'flash' | 'sram' | 'regs' | 'eeprom' | 'decoder' | 'alu' | 'control' | 'iobus' | 'periph' | 'analog' | 'clock' | 'nvm';

export interface Block {
  id: string;
  kind: BlockKind;
  label: string;
  sub: string;
  x: number;
  y: number;
  w: number;
  h: number;
  /** Register group (spec.groups) for peripheral blocks. */
  group?: string;
}

export interface Pad {
  pin: PinSpec;
  x: number;
  y: number;
  /** Pad size (square). */
  s: number;
  side: 'top' | 'bottom' | 'left' | 'right';
}

export interface Bus {
  id: 'pbus' | 'dbus' | 'iobus';
  label: string;
  points: [number, number][];
}

export interface Floorplan {
  w: number;
  h: number;
  /** Inner core area (inside the pad ring). */
  core: { x: number; y: number; w: number; h: number };
  blocks: Block[];
  pads: Pad[];
  buses: Bus[];
}

const ANALOG_GROUPS = new Set(['ADC', 'AC', 'VLM']);
const SKIP_GROUPS = new Set(['CPU', 'NVM']);

export function buildFloorplan(spec: AvrDeviceSpec): Floorplan {
  // Measured die size when known; otherwise an estimate that grows with the memories.
  const area = 1.3e6 * (1 + (spec.flashSize / 1024) * 0.18 + (spec.sramSize / 1024) * 0.5 + (spec.eepromSize / 1024) * 0.1);
  const W = spec.die?.widthUm ?? Math.round(Math.sqrt(area * 1.35));
  const H = spec.die?.heightUm ?? Math.round(area / Math.sqrt(area * 1.35));
  const ring = Math.min(W, H) * 0.16;
  const core = { x: ring, y: ring, w: W - 2 * ring, h: H - 2 * ring };
  const gap = Math.min(W, H) * 0.012;
  const blocks: Block[] = [];
  const add = (b: Block) => blocks.push({ ...b, x: b.x + gap / 2, y: b.y + gap / 2, w: b.w - gap, h: b.h - gap });

  // Row A: program memory | data memory + register file.
  const rowA = core.h * 0.47;
  const rowB = core.h * 0.23;
  const rowC = core.h - rowA - rowB;
  const flashW = core.w * 0.56;
  const words = spec.flashSize / 2;
  const eepromW = spec.eepromSize ? core.w * 0.12 : 0;
  add({ id: 'flash', kind: 'flash', label: 'FLASH', sub: `${spec.flashSize} B program memory (${words} words)`, x: core.x, y: core.y, w: flashW - eepromW, h: rowA });
  if (eepromW) add({ id: 'eeprom', kind: 'eeprom', label: 'EEPROM', sub: `${spec.eepromSize} B`, x: core.x + flashW - eepromW, y: core.y, w: eepromW, h: rowA });
  const rightX = core.x + flashW;
  const rightW = core.w - flashW;
  add({ id: 'sram', kind: 'sram', label: 'SRAM', sub: `${spec.sramSize} B data memory`, x: rightX, y: core.y, w: rightW, h: rowA * 0.48 });
  const regCount = spec.features & 1 ? 16 : 32;
  add({ id: 'regs', kind: 'regs', label: 'REGISTER FILE', sub: `${regCount} x 8-bit (${regCount === 16 ? 'R16-R31' : 'R0-R31'})`, x: rightX, y: core.y + rowA * 0.48, w: rightW, h: rowA * 0.52 });

  // Row B: CPU core.
  const yB = core.y + rowA;
  const cpu: [string, BlockKind, string, string, number][] = [
    ['decoder', 'decoder', 'INSTRUCTION DECODER', 'fetch / decode', 0.3],
    ['alu', 'alu', 'ALU', '8-bit arithmetic & logic', 0.22],
    ['control', 'control', 'CPU CONTROL', 'PC  SP  SREG', 0.28],
    ['iobus', 'iobus', 'I/O & INTERRUPTS', `${spec.vectors.length} vectors`, 0.2],
  ];
  let x = core.x;
  for (const [id, kind, label, sub, f] of cpu) {
    add({ id, kind, label, sub, x, y: yB, w: core.w * f, h: rowB });
    x += core.w * f;
  }

  // Row C: peripherals, oscillators, NVM controller.
  const yC = yB + rowB;
  const groups = spec.groups.filter((g) => !SKIP_GROUPS.has(g.name));
  const tail: Block[] = [
    { id: 'clock', kind: 'clock', label: 'OSCILLATORS', sub: `${(spec.clock.internalHz / 1e6).toFixed(0)} MHz RC / ${(spec.clock.slowHz / 1e3).toFixed(0)} kHz`, x: 0, y: 0, w: 0, h: 0, group: 'CPU' },
    { id: 'nvm', kind: 'nvm', label: 'NVM / FUSES', sub: 'controller, signature', x: 0, y: 0, w: 0, h: 0, group: 'NVM' },
  ];
  const cells = [...groups.map((g): Block => ({ id: `p-${g.name}`, kind: ANALOG_GROUPS.has(g.name) ? 'analog' : 'periph', label: g.name, sub: g.desc, x: 0, y: 0, w: 0, h: 0, group: g.name })), ...tail];
  // Two rows when there are many peripherals.
  const perRow = cells.length > 7 ? Math.ceil(cells.length / 2) : cells.length;
  const rows = Math.ceil(cells.length / perRow);
  cells.forEach((c, i) => {
    const r = Math.floor(i / perRow);
    const inRow = r === rows - 1 ? cells.length - perRow * (rows - 1) : perRow;
    const k = i - r * perRow;
    const cw = core.w / inRow;
    add({ ...c, x: core.x + k * cw, y: yC + (rowC / rows) * r, w: cw, h: rowC / rows });
  });

  // Bond pads: dual-row packages put pins 1..n/2 along the bottom edge (left to right) and the
  // rest along the top edge (right to left), matching the package's counter-clockwise numbering.
  const pins = spec.pins;
  const n = pins.length;
  const s = Math.min(ring * 0.62, 110);
  const pads: Pad[] = [];
  const quad = /QFP|QFN|MLF|PLCC/i.test(spec.package);
  if (!quad) {
    const half = Math.ceil(n / 2);
    const place = (i: number, count: number) => ring + (core.w * (i + 0.5)) / count;
    pins.forEach((pin, i) => {
      if (i < half) pads.push({ pin, x: place(i, half), y: H - ring / 2, s, side: 'bottom' });
      else pads.push({ pin, x: place(n - 1 - i, n - half), y: ring / 2, s, side: 'top' });
    });
  } else {
    const side = Math.ceil(n / 4);
    pins.forEach((pin, i) => {
      const k = i % side;
      const sideIdx = Math.floor(i / side);
      const t = (k + 0.5) / side;
      if (sideIdx === 0) pads.push({ pin, x: ring / 2, y: ring + core.h * t, s, side: 'left' });
      else if (sideIdx === 1) pads.push({ pin, x: ring + core.w * t, y: H - ring / 2, s, side: 'bottom' });
      else if (sideIdx === 2) pads.push({ pin, x: W - ring / 2, y: H - ring - core.h * t, s, side: 'right' });
      else pads.push({ pin, x: W - ring - core.w * t, y: ring / 2, s, side: 'top' });
    });
  }

  // Buses (centre lines): program bus flash -> decoder, data bus regs/SRAM <-> ALU, I/O bus.
  const b = (id: string) => blocks.find((x) => x.id === id)!;
  const fl = b('flash');
  const dec = b('decoder');
  const alu = b('alu');
  const regs = b('regs');
  const io = b('iobus');
  const ctr = b('control');
  const yBus = yB + gap * 0.1;
  const yIo = yC + gap * 0.1;
  const buses: Bus[] = [
    { id: 'pbus', label: 'Program bus (16-bit)', points: [[fl.x + fl.w * 0.35, fl.y + fl.h], [dec.x + dec.w * 0.35, yBus + rowB * 0.15]] },
    { id: 'dbus', label: 'Data bus (8-bit)', points: [[regs.x + regs.w * 0.3, regs.y + regs.h], [regs.x + regs.w * 0.3, yBus], [alu.x + alu.w * 0.5, yBus], [ctr.x + ctr.w * 0.5, yBus], [b('sram').x + b('sram').w * 0.8, yBus], [b('sram').x + b('sram').w * 0.8, b('sram').y + b('sram').h]] },
    { id: 'iobus', label: 'I/O bus', points: [[core.x + gap, yIo], [io.x + io.w * 0.5, yIo], [io.x + io.w * 0.5, io.y + io.h * 0.5], [core.x + core.w - gap, yIo]] },
  ];
  return { w: W, h: H, core, blocks, pads, buses };
}

/** Block or pad under a die position (µm). */
export function hitTest(plan: Floorplan, x: number, y: number): { block?: Block; pad?: Pad } | null {
  const pad = plan.pads.find((p) => Math.abs(p.x - x) <= p.s / 2 && Math.abs(p.y - y) <= p.s / 2);
  if (pad) return { pad };
  const block = plan.blocks.find((b) => x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h);
  return block ? { block } : null;
}
