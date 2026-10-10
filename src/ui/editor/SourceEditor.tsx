/**
 * Source editor (CodeMirror 6): one EditorView, one EditorState per document (keeps undo
 * history per file), breakpoint margin, current-statement highlight, build diagnostics,
 * hover info for registers/symbols and assembly completion.
 */
import { useEffect, useRef, type JSX } from 'react';
import { Compartment, EditorSelection, EditorState, RangeSet, StateEffect, StateField, type Extension } from '@codemirror/state';
import {
  Decoration, EditorView, GutterMarker, crosshairCursor, drawSelection, dropCursor, gutter, highlightActiveLine, highlightActiveLineGutter,
  highlightSpecialChars, hoverTooltip, keymap, lineNumbers, rectangularSelection, type DecorationSet,
} from '@codemirror/view';
import { defaultKeymap, history, historyKeymap, indentLess, indentMore, redo, selectAll, undo } from '@codemirror/commands';
import { bracketMatching, getIndentUnit, indentOnInput, indentUnit, syntaxHighlighting } from '@codemirror/language';
import { gotoLine, highlightSelectionMatches, openSearchPanel, search, searchKeymap } from '@codemirror/search';
import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap } from '@codemirror/autocomplete';
import { lintGutter, setDiagnostics, type Diagnostic as CmDiagnostic } from '@codemirror/lint';
import { cpp } from '@codemirror/lang-cpp';
import { asmCompletions, avrAsm, deviceWords, vsHighlight } from './avrLanguage';
import { machineCodeSupport } from './mcLanguage';
import { editorStates, initialTexts } from './docText';
import { setEditorApi } from './editorApi';
import { addSymbol, clearOutline, focusSymbol, outlineKind, publishOutline, revealEffect, setSymbolView, symbolView } from './symbolView';
import { markDirty } from '../services/files';
import { pcToSource, resolvedSourcePc, sameFile } from '../services/debugInfo';
import { docKey, toggleSourceBreakpoint, useWorkspace, type Doc } from '../state/workspace';
import { useSim } from '../state/sim';
import { useSettings } from '../state/settings';
import { hex } from '../format';
import { instructionSet } from '../backend/api';
import { avrCore, type InsnInfo } from '../backend/types';

/** Tab: pads with spaces to the next tab stop at the cursor; with a selection, indents the lines. */
const softTab = (view: EditorView): boolean => {
  const { state } = view;
  if (state.selection.ranges.some((r) => !r.empty)) return indentMore(view);
  const unit = getIndentUnit(state);
  view.dispatch(state.changeByRange((r) => {
    const line = state.doc.lineAt(r.head);
    let col = 0;
    for (let i = line.from; i < r.head; i++) col = line.text.charCodeAt(i - line.from) === 9 ? col + state.tabSize - (col % state.tabSize) : col + 1;
    const pad = ' '.repeat(unit - (col % unit));
    return { changes: { from: r.head, insert: pad }, range: EditorSelection.cursor(r.head + pad.length) };
  }), { scrollIntoView: true, userEvent: 'input' });
  return true;
};

// ------------------------------------------------------------------ breakpoint + exec margin

class BpMarker extends GutterMarker {
  constructor(readonly resolved: boolean, readonly enabled: boolean) {
    super();
  }
  override eq(o: BpMarker) {
    return o.resolved === this.resolved && o.enabled === this.enabled;
  }
  override toDOM() {
    const d = document.createElement('div');
    d.className = `cm-bp${this.resolved ? '' : ' unresolved'}${this.enabled ? '' : ' disabled'}`;
    return d;
  }
}

class ExecMarker extends GutterMarker {
  override toDOM() {
    const d = document.createElement('div');
    d.className = 'cm-exec-arrow';
    return d;
  }
}
const execMarker = new ExecMarker();

interface BpInfo {
  line: number;
  resolved: boolean;
  enabled: boolean;
}

const setBps = StateEffect.define<BpInfo[]>();
const setExec = StateEffect.define<number | null>();

const bpField = StateField.define<RangeSet<GutterMarker>>({
  create: () => RangeSet.empty,
  update(set, tr) {
    for (const e of tr.effects) {
      if (e.is(setBps)) {
        const doc = tr.state.doc;
        const ranges = e.value
          .filter((b) => b.line >= 1 && b.line <= doc.lines)
          .sort((a, b) => a.line - b.line)
          .map((b) => new BpMarker(b.resolved, b.enabled).range(doc.line(b.line).from));
        return RangeSet.of(ranges);
      }
    }
    return tr.docChanged ? set.map(tr.changes) : set;
  },
});

const execField = StateField.define<{ line: number | null; deco: DecorationSet; marks: RangeSet<GutterMarker> }>({
  create: () => ({ line: null, deco: Decoration.none, marks: RangeSet.empty }),
  update(v, tr) {
    for (const e of tr.effects) {
      if (e.is(setExec)) {
        const line = e.value;
        if (line === null || line < 1 || line > tr.state.doc.lines) return { line: null, deco: Decoration.none, marks: RangeSet.empty };
        const from = tr.state.doc.line(line).from;
        return { line, deco: Decoration.set([Decoration.line({ class: 'cm-exec-line' }).range(from)]), marks: RangeSet.of([execMarker.range(from)]) };
      }
    }
    return tr.docChanged ? { line: v.line, deco: v.deco.map(tr.changes), marks: v.marks.map(tr.changes) } : v;
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.deco),
});

let currentDocId: string | null = null;

const bpGutter = gutter({
  class: 'cm-bp-gutter',
  markers: (view) => RangeSet.join([view.state.field(bpField), view.state.field(execField).marks]),
  initialSpacer: () => new BpMarker(true, true),
  domEventHandlers: {
    mousedown(view, line) {
      const doc = useWorkspace.getState().docs.find((d) => d.id === currentDocId);
      if (!doc) return false;
      toggleSourceBreakpoint(docKey(doc), view.state.doc.lineAt(line.from).number);
      return true;
    },
  },
});

// ------------------------------------------------------------------ hover info

// Instruction reference per device (lower-case mnemonic -> its operand forms), loaded on demand.
let insnHelpDevice = '';
let insnHelp = new Map<string, InsnInfo[]>();
function instructionHelp(deviceId: string): Map<string, InsnInfo[]> {
  if (insnHelpDevice !== deviceId) {
    insnHelpDevice = deviceId;
    insnHelp = new Map();
    instructionSet(deviceId)
      .then((rows) => {
        if (insnHelpDevice !== deviceId) return;
        const m = new Map<string, InsnInfo[]>();
        for (const r of rows) {
          const k = r.mnemonic.toLowerCase();
          const list = m.get(k);
          if (list) list.push(r);
          else m.set(k, [r]);
        }
        insnHelp = m;
      })
      .catch(() => {});
  }
  return insnHelp;
}

function insnHoverDom(forms: InsnInfo[]): HTMLElement {
  const f = forms[0];
  const dom = document.createElement('div');
  dom.className = 'cm-hover-info cm-hover-insn';
  const add = (tag: string, text: string, cls?: string) => {
    const el = document.createElement(tag);
    el.textContent = text;
    if (cls) el.className = cls;
    dom.appendChild(el);
  };
  add('b', `${f.mnemonic} — ${f.summary}${f.aliasOf ? ` (alias of ${f.aliasOf})` : ''}`);
  add('div', forms.map((x) => `${x.mnemonic} ${x.operands}`.trim() + `    ${x.operation}`).join('\n'), 'mono');
  const cyc = [...new Set(forms.map((x) => x.cycles))].join('/');
  add('div', `Flags: ${f.flags || '-'}   Cycles: ${cyc}   Words: ${[...new Set(forms.map((x) => x.words))].join('/')}`, 'dim');
  if (f.usage) add('div', f.usage, 'insn-usage');
  if (f.example) add('pre', f.example, 'mono insn-example');
  return dom;
}

const hoverInfo = hoverTooltip((view, pos) => {
  const line = view.state.doc.lineAt(pos);
  const text = line.text;
  let s = pos - line.from;
  let e = s;
  while (s > 0 && /\w/.test(text[s - 1])) s--;
  while (e < text.length && /\w/.test(text[e])) e++;
  if (s === e) return null;
  const word = text.slice(s, e);
  const sim = useSim.getState();
  const spec = sim.spec;
  const st = sim.state;
  let info: string | null = null;
  // Instruction mnemonic: first word of the statement (after an optional label).
  if (spec && /^\s*(?:[A-Za-z_.]\w*:\s*)?$/.test(text.slice(0, s))) {
    const forms = instructionHelp(spec.id).get(word.toLowerCase());
    if (forms) return { pos: line.from + s, end: line.from + e, above: true, create: () => ({ dom: insnHoverDom(forms) }) };
  }
  const reg = /^r(\d{1,2})$/i.exec(word);
  if (reg && Number(reg[1]) < 32) {
    info = st ? `${word.toUpperCase()} = ${hex(avrCore(st).regs[Number(reg[1])])} (${avrCore(st).regs[Number(reg[1])]})` : `Register ${word}`;
  } else if (spec) {
    const r = spec.registers.find((x) => x.name === word.toUpperCase());
    if (r) {
      const io = r.addr - spec.ioBase;
      const val = st ? ` = ${hex(st.data[r.addr])}` : '';
      info = `${r.name} (I/O ${hex(io)}, data ${hex(r.addr, 4)})${val}\n${r.desc}`;
      if (r.bits.length) info += `\nBits: ${r.bits.map((b) => b.name).join(' ')}`;
    } else {
      const bitOwner = spec.registers.find((x) => x.bits.some((b) => b.name === word.toUpperCase() || (b.mask !== 0 && word.toUpperCase().startsWith(b.name))));
      const sym = useWorkspace.getState().build?.program.symbols.find((x) => x.name.toLowerCase() === word.toLowerCase());
      if (sym) {
        const val = sym.space === 'data' && st && sym.address < st.data.length ? ` = ${hex(st.data[sym.address])}` : '';
        info = `${sym.name}: ${sym.kind} at ${sym.space} ${hex(sym.address, 4)}${sym.size ? ` (${sym.size} bytes)` : ''}${val}`;
      } else if (bitOwner) {
        const b = bitOwner.bits.find((x) => x.name === word.toUpperCase());
        if (b) info = `${b.name} — bit mask ${hex(b.mask)} in ${bitOwner.name}${b.desc ? `\n${b.desc}` : ''}`;
      }
    }
  }
  if (!info) return null;
  return {
    pos: line.from + s,
    end: line.from + e,
    above: true,
    create: () => {
      const dom = document.createElement('div');
      dom.className = 'cm-hover-info';
      dom.textContent = info;
      return { dom };
    },
  };
});

// ------------------------------------------------------------------ theme

const theme = EditorView.theme({
  '&': { height: '100%', fontSize: 'var(--editor-fs, 13px)', backgroundColor: '#fff' },
  '.cm-scroller': { fontFamily: 'var(--font-mono)', lineHeight: '1.42' },
  '.cm-content': { caretColor: '#000', padding: '2px 0' },
  '.cm-gutters': { backgroundColor: '#fff', borderRight: '1px solid #e0e6ef', color: '#2b91af' },
  '.cm-lineNumbers .cm-gutterElement': { padding: '0 8px 0 6px', minWidth: '34px' },
  '.cm-bp-gutter': { width: '18px', backgroundColor: '#eceff4', borderRight: '1px solid #dde3ec' },
  '.cm-bp-gutter .cm-gutterElement': { position: 'relative', cursor: 'pointer' },
  '.cm-activeLine': { backgroundColor: 'rgba(219, 234, 252, 0.45)' },
  '.cm-activeLineGutter': { backgroundColor: 'transparent', color: '#1c4f9c', fontWeight: '600' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': { backgroundColor: '#add6ff !important' },
  '.cm-selectionMatch': { backgroundColor: '#e2eefc' },
  '.cm-matchingBracket': { backgroundColor: '#dbe7f3', outline: '1px solid #a5c3e3' },
  '.cm-tooltip': { border: '1px solid #767676', borderRadius: '3px', background: 'linear-gradient(#fff, #e4e5f0)', boxShadow: '1px 2px 3px rgba(0,0,0,.25)' },
  '.cm-tooltip-autocomplete > ul > li[aria-selected]': { background: 'linear-gradient(#dcebfc, #c1dbfc)', color: '#000' },
  '.cm-panels': { backgroundColor: '#f0f0f0', borderColor: '#a5b4c8' },
  '.cm-panel.cm-search': { padding: '4px 6px', fontFamily: 'var(--font-ui)' },
  '.cm-panel input, .cm-panel button': { fontFamily: 'var(--font-ui)', fontSize: '12px' },
  '.cm-button': { backgroundImage: 'linear-gradient(#f2f2f2, #ebebeb 49%, #dddddd 50%, #cfcfcf)', border: '1px solid #707070', borderRadius: '3px' },
  '.cm-textfield': { border: '1px solid #abadb3', borderRadius: '2px' },
  '.cm-mc-hint': { color: '#5a7f5a', marginLeft: '2.5em', fontStyle: 'italic', pointerEvents: 'none' },
  '.cm-mc-hint.invalid': { color: '#c0392b' },
});

// ------------------------------------------------------------------ state factory

const languageConf = new Compartment();
const fontConf = new Compartment();

function languageFor(doc: Doc): Extension {
  if (doc.language === 'mc') return [machineCodeSupport(), outlineKind.of('none')];
  if (doc.language === 'asm') return [avrAsm, autocompletion({ override: [asmCompletions] }), outlineKind.of('asm')];
  // GNU assembler sources (.S) use C highlighting but have assembly labels.
  return [cpp(), autocompletion(), outlineKind.of(doc.language === 'c' ? 'c' : 'asm')];
}

function createState(doc: Doc, text: string): EditorState {
  return EditorState.create({
    doc: text,
    extensions: [
      bpField,
      execField,
      bpGutter,
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightSpecialChars(),
      history(),
      drawSelection(),
      dropCursor(),
      EditorState.allowMultipleSelections.of(true),
      indentOnInput(),
      indentUnit.of(doc.language === 'asm' ? '        ' : '    '),
      syntaxHighlighting(vsHighlight),
      bracketMatching(),
      closeBrackets(),
      rectangularSelection(),
      crosshairCursor(),
      highlightActiveLine(),
      highlightSelectionMatches(),
      search({ top: true }),
      lintGutter(),
      hoverInfo,
      symbolView(),
      languageConf.of(languageFor(doc)),
      fontConf.of([]),
      keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...searchKeymap, ...historyKeymap, ...completionKeymap, { key: 'Tab', run: softTab, shift: indentLess }]),
      theme,
      EditorView.updateListener.of((u) => {
        if (u.docChanged && currentDocId) {
          // Builds read the text from here: keep it in step with the view.
          editorStates.set(currentDocId, u.state);
          markDirty(currentDocId);
          remapBreakpoints(u.startState, u.state, u.changes);
        }
        if (u.selectionSet || u.docChanged) {
          const head = u.state.selection.main.head;
          const l = u.state.doc.lineAt(head);
          useWorkspace.setState({ cursor: { line: l.number, col: head - l.from + 1 } });
        }
        if (useSettings.getState().symbolView && (u.docChanged || u.selectionSet || u.transactions.some((t) => t.effects.length))) publishOutline(u.view);
      }),
    ],
  });
}

/** Keeps source breakpoints on the same code when lines are inserted/removed above them. */
function remapBreakpoints(before: EditorState, after: EditorState, changes: import('@codemirror/state').ChangeSet): void {
  const ws = useWorkspace.getState();
  const doc = ws.docs.find((d) => d.id === currentDocId);
  if (!doc) return;
  const key = docKey(doc);
  let changed = false;
  const bps = ws.breakpoints.map((b) => {
    if (b.kind !== 'source' || b.file !== key || b.line > before.doc.lines) return b;
    const pos = changes.mapPos(before.doc.line(b.line).from, 1);
    const line = after.doc.lineAt(pos).number;
    if (line === b.line) return b;
    changed = true;
    return { ...b, line };
  });
  if (changed) useWorkspace.setState({ breakpoints: bps });
}

// ------------------------------------------------------------------ component

export function SourceEditor({ doc }: { doc: Doc }): JSX.Element {
  const host = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const fontSize = useSettings((s) => s.editorFontSize);

  // Create the view once.
  useEffect(() => {
    const view = new EditorView({ parent: host.current! });
    viewRef.current = view;
    setEditorApi({
      undo: () => undo(view),
      redo: () => redo(view),
      cut: () => {
        void navigator.clipboard.writeText(view.state.sliceDoc(view.state.selection.main.from, view.state.selection.main.to));
        view.dispatch(view.state.replaceSelection(''));
      },
      copy: () => void navigator.clipboard.writeText(view.state.sliceDoc(view.state.selection.main.from, view.state.selection.main.to)),
      paste: () => void navigator.clipboard.readText().then((t) => view.dispatch(view.state.replaceSelection(t))),
      selectAll: () => selectAll(view),
      find: () => openSearchPanel(view),
      replace: () => openSearchPanel(view),
      gotoLine: () => gotoLine(view),
      cursorLine: () => view.state.doc.lineAt(view.state.selection.main.head).number,
      focus: () => view.focus(),
      insertText: (text) => {
        view.dispatch(view.state.replaceSelection(text));
        view.focus();
      },
      focusSymbol: (index) => focusSymbol(view, index),
      addSymbol: (name) => addSymbol(view, name),
    });
    const unsubSettings = useSettings.subscribe((s, p) => {
      if (s.symbolView === p.symbolView || !currentDocId) return;
      setSymbolView(view, s.symbolView);
      publishOutline(view, true);
    });
    return () => {
      unsubSettings();
      clearOutline();
      if (currentDocId) editorStates.set(currentDocId, view.state);
      currentDocId = null;
      setEditorApi(null);
      view.destroy();
    };
  }, []);

  // Swap document state when the active document changes.
  useEffect(() => {
    const view = viewRef.current!;
    if (currentDocId && currentDocId !== doc.id) editorStates.set(currentDocId, view.state);
    let st = editorStates.get(doc.id);
    if (!st) {
      st = createState(doc, initialTexts.get(doc.id) ?? '');
      editorStates.set(doc.id, st);
    }
    currentDocId = doc.id;
    view.setState(st);
    syncDecorations(view, doc);
    const symView = useSettings.getState().symbolView;
    setSymbolView(view, symView);
    if (symView) publishOutline(view, true);
    view.focus();
  }, [doc.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // Language follows file renames (Save As .c).
  useEffect(() => {
    viewRef.current?.dispatch({ effects: languageConf.reconfigure(languageFor(doc)) });
  }, [doc.language]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    host.current?.style.setProperty('--editor-fs', `${fontSize}px`);
  }, [fontSize]);

  // Breakpoints, diagnostics and execution line follow the stores.
  useEffect(() => {
    const update = () => {
      const v = viewRef.current;
      const d = useWorkspace.getState().docs.find((x) => x.id === currentDocId);
      if (v && d) syncDecorations(v, d);
    };
    const unsubWs = useWorkspace.subscribe((s, p) => {
      if (s.breakpoints !== p.breakpoints || s.diagnostics !== p.diagnostics || s.build !== p.build) update();
      if (s.goto && s.goto !== p.goto && s.goto.docId === currentDocId) {
        const v = viewRef.current!;
        const line = Math.min(Math.max(1, s.goto.line), v.state.doc.lines);
        const pos = v.state.doc.line(line).from;
        v.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: 'center' }) });
        v.focus();
      }
    });
    const unsubSim = useSim.subscribe((s, p) => {
      if (s.running !== p.running || (s.state?.pc !== p.state?.pc && !s.running) || s.revealSeq !== p.revealSeq) update();
    });
    return () => {
      unsubWs();
      unsubSim();
    };
  }, []);

  // Device register/bit names for highlighting.
  const spec = useSim((s) => s.spec);
  useEffect(() => {
    deviceWords.registers = new Set(spec?.registers.map((r) => r.name) ?? []);
    const bits = new Set<string>();
    for (const r of spec?.registers ?? []) {
      for (const b of r.bits) {
        const n = popcount(b.mask);
        if (n === 1) bits.add(b.name);
        else for (let i = 0; i < n; i++) bits.add(/\d$/.test(b.name) ? `${b.name}${i}` : `${b.name}${i}`);
      }
    }
    deviceWords.bits = bits;
  }, [spec]);

  return <div className="editor-host" ref={host} style={{ height: '100%' }} />;
}

function popcount(m: number): number {
  let c = 0;
  for (; m; m &= m - 1) c++;
  return c;
}

function syncDecorations(view: EditorView, doc: Doc): void {
  const ws = useWorkspace.getState();
  const sim = useSim.getState();
  const key = docKey(doc);
  const program = ws.build?.program ?? null;
  const bps: BpInfo[] = ws.breakpoints
    .filter((b): b is Extract<typeof b, { kind: 'source' }> => b.kind === 'source' && b.file === key)
    .map((b) => ({ line: b.line, enabled: b.enabled, resolved: !program || resolvedSourcePc(program, b.file, b.line) >= 0 }));

  let execLine: number | null = null;
  if (!sim.running && sim.state && program) {
    const loc = pcToSource(program, sim.state.pc);
    if (loc && sameFile(loc.file, key)) execLine = loc.line;
  }

  const total = view.state.doc.length;
  const diags: CmDiagnostic[] = ws.diagnostics
    .filter((d) => d.line > 0 && (!d.file || sameFile(d.file, key)))
    .filter((d) => d.line <= view.state.doc.lines)
    .map((d) => {
      const line = view.state.doc.line(d.line);
      const from = Math.min(total, line.from + Math.max(0, d.column - 1));
      return { from, to: Math.max(from, line.to), severity: d.severity === 'info' ? 'info' : d.severity, message: d.message };
    });

  view.dispatch(setDiagnostics(view.state, diags));
  const effects: StateEffect<unknown>[] = [setBps.of(bps), setExec.of(execLine)];
  // Symbol View: stopping in another symbol shows that symbol.
  const reveal = execLine !== null ? revealEffect(view.state, view.state.doc.line(execLine).from) : null;
  if (reveal) effects.push(reveal);
  view.dispatch({ effects });
  if (execLine !== null) {
    const pos = view.state.doc.line(execLine).from;
    const vp = view.viewport;
    if (pos < vp.from || pos > vp.to) view.dispatch({ effects: EditorView.scrollIntoView(pos, { y: 'center' }) });
  }
}
