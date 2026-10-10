/**
 * Workspace state: open documents, build results, breakpoints and the output log.
 */
import { create } from 'zustand';
import type { Arch, Diagnostic, LoadedProgram } from '../backend/types';
import { SymbolIndex, sourceToPc } from '../services/debugInfo';
import { forgetDoc, initialTexts } from '../editor/docText';

export type DocLanguage = 'asm' | 'c' | 'gas' | 'mc';

export interface Doc {
  id: string;
  name: string;
  path: string | null;
  language: DocLanguage;
  dirty: boolean;
}

export type Breakpoint =
  | { id: number; kind: 'source'; file: string; line: number; enabled: boolean }
  | { id: number; kind: 'address'; pc: number; enabled: boolean };

export type OutputLevel = 'info' | 'warning' | 'error' | 'success' | 'cmd';

export interface OutputLine {
  id: number;
  level: OutputLevel;
  text: string;
  file?: string;
  line?: number;
}

export interface BuildInfo {
  program: LoadedProgram;
  /** Document the program was built from (null for imported HEX/ELF). */
  docId: string | null;
  label: string;
  symbols: SymbolIndex;
  time: number;
  /** Architecture of the device the program was loaded for (selects the pc unit). */
  arch: Arch;
}

interface WorkspaceStore {
  docs: Doc[];
  activeDocId: string | null;
  breakpoints: Breakpoint[];
  output: OutputLine[];
  build: BuildInfo | null;
  building: boolean;
  diagnostics: Diagnostic[];
  cursor: { line: number; col: number };
  /** Bumped to ask the dock to bring the Output panel to front. */
  showOutputSeq: number;
  /** Request to scroll the editor to a location (consumed by the editor). */
  goto: { docId: string; line: number; seq: number } | null;
  /** Request to scroll the disassembly to a program counter (native unit). */
  disasmGoto: { pc: number; seq: number } | null;
}

export const useWorkspace = create<WorkspaceStore>(() => ({
  docs: [],
  activeDocId: null,
  breakpoints: [],
  output: [],
  build: null,
  building: false,
  diagnostics: [],
  cursor: { line: 1, col: 1 },
  showOutputSeq: 0,
  goto: null,
  disasmGoto: null,
}));

let nextDocId = 1;
let nextBpId = 1;
let nextOutId = 1;

export function docKey(d: Doc): string {
  return d.path ?? d.name;
}

export function languageFor(name: string): DocLanguage {
  const ext = name.slice(name.lastIndexOf('.')).toLowerCase();
  if (name.endsWith('.S') || ext === '.sx') return 'gas';
  if (ext === '.c' || ext === '.h' || ext === '.cpp' || ext === '.cc' || ext === '.hpp') return 'c';
  if (ext === '.mc') return 'mc';
  return 'asm';
}

export function addDoc(name: string, path: string | null, text: string, language = languageFor(name)): string {
  const existing = path ? useWorkspace.getState().docs.find((d) => d.path === path) : undefined;
  if (existing) {
    useWorkspace.setState({ activeDocId: existing.id });
    return existing.id;
  }
  const id = `doc${nextDocId++}`;
  initialTexts.set(id, text);
  useWorkspace.setState((s) => ({ docs: [...s.docs, { id, name, path, language, dirty: false }], activeDocId: id }));
  return id;
}

export function closeDoc(id: string): void {
  forgetDoc(id);
  useWorkspace.setState((s) => {
    const idx = s.docs.findIndex((d) => d.id === id);
    const docs = s.docs.filter((d) => d.id !== id);
    let active = s.activeDocId;
    if (active === id) active = docs[Math.min(idx, docs.length - 1)]?.id ?? null;
    return { docs, activeDocId: active };
  });
}

export function updateDoc(id: string, patch: Partial<Doc>): void {
  useWorkspace.setState((s) => ({ docs: s.docs.map((d) => (d.id === id ? { ...d, ...patch } : d)) }));
}

export function activeDoc(): Doc | undefined {
  const s = useWorkspace.getState();
  return s.docs.find((d) => d.id === s.activeDocId);
}

export function untitledName(ext: string): string {
  const names = new Set(useWorkspace.getState().docs.map((d) => d.name));
  for (let i = 1; ; i++) {
    const n = `Untitled${i}${ext}`;
    if (!names.has(n)) return n;
  }
}

// ---------------------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------------------

const MAX_OUTPUT = 2000;

export function appendOutput(level: OutputLevel, text: string, loc?: { file: string; line: number }): void {
  useWorkspace.setState((s) => {
    const out = s.output.length >= MAX_OUTPUT ? s.output.slice(s.output.length - MAX_OUTPUT + 1) : s.output.slice();
    out.push({ id: nextOutId++, level, text, file: loc?.file, line: loc?.line });
    return { output: out };
  });
}

export function clearOutput(): void {
  useWorkspace.setState({ output: [] });
}

export function showOutput(): void {
  useWorkspace.setState((s) => ({ showOutputSeq: s.showOutputSeq + 1 }));
}

// ---------------------------------------------------------------------------------------
// Breakpoints
// ---------------------------------------------------------------------------------------

export function toggleSourceBreakpoint(file: string, line: number): void {
  useWorkspace.setState((s) => {
    const hit = s.breakpoints.find((b) => b.kind === 'source' && b.file === file && b.line === line);
    if (hit) return { breakpoints: s.breakpoints.filter((b) => b !== hit) };
    return { breakpoints: [...s.breakpoints, { id: nextBpId++, kind: 'source', file, line, enabled: true }] };
  });
}

export function toggleAddressBreakpoint(pc: number): void {
  useWorkspace.setState((s) => {
    const resolved = resolvedPcMap(s.breakpoints, s.build?.program ?? null, s.build?.arch);
    const owners = resolved.get(pc);
    if (owners && owners.length) return { breakpoints: s.breakpoints.filter((b) => !owners.includes(b.id)) };
    return { breakpoints: [...s.breakpoints, { id: nextBpId++, kind: 'address', pc, enabled: true }] };
  });
}

export function setBreakpointEnabled(id: number, enabled: boolean): void {
  useWorkspace.setState((s) => ({ breakpoints: s.breakpoints.map((b) => (b.id === id ? { ...b, enabled } : b)) }));
}

export function removeBreakpoint(id: number): void {
  useWorkspace.setState((s) => ({ breakpoints: s.breakpoints.filter((b) => b.id !== id) }));
}

export function clearBreakpoints(): void {
  useWorkspace.setState({ breakpoints: [] });
}

/** Resolved program counter (native unit) per breakpoint (-1 when unresolved). */
export function resolveBreakpoint(b: Breakpoint, program: LoadedProgram | null, arch: Arch = 'avr'): number {
  if (b.kind === 'address') return b.pc;
  return sourceToPc(program, b.file, b.line, arch).pc;
}

/** pc -> breakpoint ids. */
export function resolvedPcMap(bps: Breakpoint[], program: LoadedProgram | null, arch: Arch = 'avr'): Map<number, number[]> {
  const map = new Map<number, number[]>();
  for (const b of bps) {
    const pc = resolveBreakpoint(b, program, arch);
    if (pc < 0) continue;
    const arr = map.get(pc);
    if (arr) arr.push(b.id);
    else map.set(pc, [b.id]);
  }
  return map;
}

export function enabledBreakpointPcs(): number[] {
  const s = useWorkspace.getState();
  const program = s.build?.program ?? null;
  const pcs = new Set<number>();
  for (const b of s.breakpoints) {
    if (!b.enabled) continue;
    const pc = resolveBreakpoint(b, program, s.build?.arch);
    if (pc >= 0) pcs.add(pc);
  }
  return [...pcs];
}

export function setBuild(program: LoadedProgram, docId: string | null, label: string, arch: Arch = 'avr'): void {
  useWorkspace.setState({ build: { program, docId, label, symbols: new SymbolIndex(program), time: Date.now(), arch } });
}

export function requestGoto(docId: string, line: number): void {
  useWorkspace.setState((s) => ({ activeDocId: docId, goto: { docId, line, seq: (s.goto?.seq ?? 0) + 1 } }));
}

export function requestDisasmGoto(pc: number): void {
  useWorkspace.setState((s) => ({ disasmGoto: { pc, seq: (s.disasmGoto?.seq ?? 0) + 1 } }));
}
