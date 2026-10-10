/** Address <-> source line / symbol lookups over a LoadedProgram. */
import { bytesToPc, pcToBytes, type Arch, type LoadedProgram, type ProgramSymbol } from '../backend/types';

export function baseName(p: string): string {
  const i = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'));
  return i >= 0 ? p.slice(i + 1) : p;
}

/** Whether a program file entry refers to the given document key (path or name). */
export function sameFile(programFile: string, docKey: string): boolean {
  if (programFile === docKey) return true;
  const a = programFile.replace(/\\/g, '/');
  const b = docKey.replace(/\\/g, '/');
  return a === b || baseName(a) === baseName(b);
}

export interface SourceLoc {
  file: string;
  line: number;
}

/** Source location for a program counter in the architecture's native unit (AVR words, ARM bytes), or null. */
export function pcToSource(p: LoadedProgram | null | undefined, pc: number, arch: Arch = 'avr'): SourceLoc | null {
  if (!p || p.lines.length === 0) return null;
  const addr = pcToBytes(arch, pc);
  const rows = p.lines;
  let lo = 0;
  let hi = rows.length - 1;
  let idx = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (rows[mid].address <= addr) {
      idx = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  if (idx < 0) return null;
  // Several rows can share an address (`for(;;)` and its first statement, or a call site and
  // inlined header code). Like the simulator's stepping: take the file of the first statement
  // row, then the last statement row from that file.
  while (idx > 0 && rows[idx - 1].address === rows[idx].address) idx--;
  let row = rows[idx];
  let file = -1;
  for (let j = idx; j < rows.length && rows[j].address === rows[idx].address; j++) {
    const r = rows[j];
    if (!r.isStmt) continue;
    if (file === -1) file = r.file;
    if (r.file === file) row = r;
  }
  // Reject addresses far past the last row of a sequence (e.g. unmapped library code).
  const next = rows[idx + 1];
  if (!next && addr - row.address > 64) return null;
  return { file: p.files[row.file] ?? '', line: row.line };
}

/** First code pc (native unit) for a source line (searching forward a few lines), or -1. */
export function sourceToPc(p: LoadedProgram | null | undefined, docKey: string, line: number, arch: Arch = 'avr'): { pc: number; line: number } {
  if (!p) return { pc: -1, line };
  const fileIdx = new Set<number>();
  p.files.forEach((f, i) => { if (sameFile(f, docKey)) fileIdx.add(i); });
  if (fileIdx.size === 0) return { pc: -1, line };
  for (let l = line; l < line + 30; l++) {
    let best = -1;
    for (const row of p.lines) if (row.line === l && fileIdx.has(row.file) && (best < 0 || row.address < best)) best = row.address;
    if (best >= 0) return { pc: bytesToPc(arch, best), line: l };
  }
  return { pc: -1, line };
}

export class SymbolIndex {
  readonly code: ProgramSymbol[];
  readonly data: ProgramSymbol[];
  private codeByAddr = new Map<number, string>();
  private dataByAddr = new Map<number, string>();

  constructor(p: LoadedProgram | null | undefined) {
    const syms = p?.symbols ?? [];
    this.code = syms.filter((s) => s.space === 'code' && (s.kind === 'func' || s.kind === 'label')).sort((a, b) => a.address - b.address);
    this.data = syms.filter((s) => s.space === 'data').sort((a, b) => a.address - b.address);
    for (const s of this.code) if (!this.codeByAddr.has(s.address) || s.kind === 'func') this.codeByAddr.set(s.address, s.name);
    for (const s of this.data) if (!this.dataByAddr.has(s.address)) this.dataByAddr.set(s.address, s.name);
  }

  codeLabel(byteAddr: number): string | undefined {
    return this.codeByAddr.get(byteAddr);
  }

  dataLabel(addr: number): string | undefined {
    return this.dataByAddr.get(addr);
  }

  /** "func+0x12" style description of a code address. */
  describeCode(byteAddr: number): string {
    let best: ProgramSymbol | undefined;
    for (const s of this.code) {
      if (s.address > byteAddr) break;
      best = s;
    }
    if (!best) return `0x${byteAddr.toString(16).toUpperCase().padStart(4, '0')}`;
    const off = byteAddr - best.address;
    return off === 0 ? best.name : `${best.name}+0x${off.toString(16).toUpperCase()}`;
  }
}

/** Program counter (native unit) a source breakpoint resolves to, or -1. */
export function resolvedSourcePc(p: LoadedProgram | null | undefined, docKey: string, line: number, arch: Arch = 'avr'): number {
  return sourceToPc(p, docKey, line, arch).pc;
}
