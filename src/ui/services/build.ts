/**
 * Build pipeline: assembles (Rust assembler) or compiles (avr-gcc via the backend) the active
 * document, reports diagnostics to the Output window/editor and loads the result into the
 * simulator. Also handles HEX/ELF import.
 */
import { buildAsm, buildC, buildMachineCode, importProgram, importProgramBytes, pickFile, programToMachineCode } from '../backend/api';
import type { BuildOutcome, Diagnostic, LoadedProgram } from '../backend/types';
import { getDocText } from '../editor/docText';
import { useSettings } from '../state/settings';
import { activeDoc, addDoc, appendOutput, clearOutput, enabledBreakpointPcs, setBuild, showOutput, untitledName, useWorkspace, type Doc } from '../state/workspace';
import { useSim } from '../state/sim';
import { sim } from './simClient';
import { baseName } from './debugInfo';
import { archOf } from '../state/devices';

/** Source text the current build was produced from (to detect stale builds). */
let builtText: string | null = null;
let builtDocId: string | null = null;

export function isBuildStale(): boolean {
  const ws = useWorkspace.getState();
  if (!ws.build) return true;
  if (ws.build.docId === null) return false; // imported image
  const doc = ws.docs.find((d) => d.id === ws.build!.docId);
  if (!doc) return false;
  return builtDocId !== doc.id || getDocText(doc.id) !== builtText;
}

function logDiagnostics(diags: Diagnostic[], defaultFile: string): void {
  for (const d of diags) {
    const file = d.file || defaultFile;
    const where = d.line ? `${baseName(file)}(${d.line}${d.column ? `,${d.column}` : ''}): ` : '';
    appendOutput(d.severity === 'info' ? 'info' : d.severity, `${where}${d.severity}: ${d.message}`, d.line ? { file, line: d.line } : undefined);
  }
}

function summarize(program: LoadedProgram, deviceId: string): string {
  const spec = useSim.getState().spec;
  const size = spec && spec.id === deviceId ? spec.flashSize : program.flash.length;
  const pct = ((program.flashUsed / size) * 100).toFixed(1);
  return `Program memory usage: ${program.flashUsed} bytes (${pct}% of ${size} bytes)`;
}

/** Builds the active document. Returns true on success. */
export async function buildActive(): Promise<boolean> {
  const doc = activeDoc();
  if (!doc) {
    appendOutput('warning', 'Nothing to build: open or create a source file first.');
    showOutput();
    return false;
  }
  return buildDoc(doc);
}

export async function buildDoc(doc: Doc): Promise<boolean> {
  const settings = useSettings.getState();
  const text = getDocText(doc.id);
  const t0 = performance.now();
  if (settings.clearOutputOnRun) clearOutput();
  useWorkspace.setState({ building: true, diagnostics: [] });
  const tool = { asm: 'MCS assembler', mc: 'machine code', c: 'avr-gcc', gas: 'avr-gcc' }[doc.language];
  appendOutput('cmd', `------ Build started: ${doc.name} (${tool}, ${settings.deviceId}) ------`);
  let outcome: BuildOutcome;
  try {
    outcome =
      doc.language === 'asm'
        ? await buildAsm(text, doc.name, doc.path, settings.deviceId)
        : doc.language === 'mc'
          ? await buildMachineCode(text, doc.name, doc.path, settings.deviceId)
          : await buildC({
            source: text,
            fileName: doc.path ?? doc.name,
            filePath: doc.path,
            deviceId: settings.deviceId,
            optimize: settings.optimize,
            extraFlags: settings.extraFlags.split(/\s+/).filter(Boolean),
            gccPath: settings.gccPath || null,
          });
  } catch (e) {
    useWorkspace.setState({ building: false });
    appendOutput('error', `Build failed: ${e instanceof Error ? e.message : String(e)}`);
    showOutput();
    return false;
  }
  if (outcome.output.trim()) for (const l of outcome.output.trimEnd().split('\n')) appendOutput('info', l);
  logDiagnostics(outcome.diagnostics, doc.path ?? doc.name);
  const errors = outcome.diagnostics.filter((d) => d.severity === 'error').length;
  const warnings = outcome.diagnostics.filter((d) => d.severity === 'warning').length;
  useWorkspace.setState({ building: false, diagnostics: outcome.diagnostics });
  const ms = (performance.now() - t0).toFixed(0);
  if (!outcome.ok || !outcome.program) {
    appendOutput('error', `Build FAILED: ${errors} error(s), ${warnings} warning(s) (${ms} ms)`);
    showOutput();
    return false;
  }
  if (outcome.deviceId && outcome.deviceId !== settings.deviceId) {
    appendOutput('info', `Device selected by source: ${outcome.deviceId}`);
    useSettings.getState().set({ deviceId: outcome.deviceId });
  }
  appendOutput('info', summarize(outcome.program, outcome.deviceId));
  appendOutput('success', `Build succeeded: 0 error(s), ${warnings} warning(s) (${ms} ms)`);
  builtText = text;
  builtDocId = doc.id;
  loadProgram(outcome.program, doc.id, doc.name, outcome.deviceId || settings.deviceId);
  return true;
}

export function loadProgram(program: LoadedProgram, docId: string | null, label: string, deviceId: string): void {
  setBuild(program, docId, label, archOf(deviceId));
  sim({ type: 'load', deviceId, program });
  sim({ type: 'setBreakpoints', pcs: enabledBreakpointPcs() });
}

/** Reports an imported image and loads it into the simulator. Returns true on success. */
function loadImported(r: BuildOutcome, name: string): boolean {
  logDiagnostics(r.diagnostics, name);
  if (!r.ok || !r.program) {
    appendOutput('error', `Import failed: ${name}`);
    showOutput();
    return false;
  }
  if (r.deviceId !== useSettings.getState().deviceId) useSettings.getState().set({ deviceId: r.deviceId });
  appendOutput('success', `Imported ${name} (${r.program.format.toUpperCase()}, ${r.program.flashUsed} bytes, ${r.program.lines.length} line-table rows)`);
  builtText = null;
  builtDocId = null;
  loadProgram(r.program, null, name, r.deviceId);
  return true;
}

/** File > Import HEX/ELF. */
export async function importHexOrElf(): Promise<void> {
  const path = await pickFile('Import Program Image', [
    { name: 'Program images', extensions: ['hex', 'ihex', 'elf', 'out', 'o'] },
    { name: 'All files', extensions: ['*'] },
  ]);
  if (!path) return;
  try {
    loadImported(await importProgram(path, useSettings.getState().deviceId), baseName(path));
  } catch (e) {
    appendOutput('error', `Import failed: ${e instanceof Error ? e.message : String(e)}`);
    showOutput();
  }
}

/** Loads a bundled prebuilt image (an example without source) onto the current device. */
export async function loadBundledImage(url: string, name: string): Promise<boolean> {
  try {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const bytes = new Uint8Array(await res.arrayBuffer());
    return loadImported(await importProgramBytes(bytes, name, useSettings.getState().deviceId), name);
  } catch (e) {
    appendOutput('error', `Could not load ${name}: ${e instanceof Error ? e.message : String(e)}`);
    showOutput();
    return false;
  }
}

/** Build > Open Program as Machine Code: the loaded image as an editable .mc document. */
export async function openProgramAsMachineCode(): Promise<void> {
  const b = useWorkspace.getState().build;
  const spec = useSim.getState().spec;
  if (!b || !spec) return;
  const labels: Record<number, string> = {};
  for (const s of b.symbols.code) if (!(s.address in labels)) labels[s.address] = s.name;
  try {
    const text = await programToMachineCode(spec.id, b.program.flash, b.program.flashUsed, labels, b.label);
    addDoc(untitledName('.mc'), null, text, 'mc');
  } catch (e) {
    appendOutput('error', `Could not convert the program: ${e instanceof Error ? e.message : String(e)}`);
    showOutput();
  }
}
