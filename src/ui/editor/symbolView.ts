/**
 * Symbol View: shows one symbol of a document at a time (an assembly label or a C function),
 * as if each were its own file. The document stays one text with real line numbers and one
 * undo history: the other symbols are hidden behind block decorations, typing and the cursor
 * stay inside the shown section, and jumps made by something other than the user's cursor
 * (go to line, breakpoints, the debugger, search, undo) switch to the section they land in.
 *
 * The outline is kept incrementally: assembly edits rescan only the changed lines, C reads the
 * top-level nodes of the (already incremental) syntax tree.
 */
import { EditorSelection, EditorState, Facet, StateEffect, StateField, type Extension, type Text, type Transaction } from '@codemirror/state';
import { Decoration, EditorView, WidgetType, type DecorationSet } from '@codemirror/view';
import { indentUnit, syntaxTree } from '@codemirror/language';
import type { SyntaxNode, Tree } from '@lezer/common';
import { create } from 'zustand';

export type OutlineKind = 'asm' | 'c' | 'none';

/** How the document's symbols are found; set together with the language. */
export const outlineKind = Facet.define<OutlineKind, OutlineKind>({ combine: (v) => v[0] ?? 'none' });

interface Sym {
  name: string;
  /** Start of the line that defines the symbol. */
  pos: number;
}

interface Outline {
  kind: OutlineKind;
  syms: Sym[];
  /** Tree the C outline was read from. */
  tree: Tree | null;
}

/** Shown part of the document: `from` to the start of the next section (`to`, null = end of file). */
interface Region {
  from: number;
  to: number | null;
}

// ------------------------------------------------------------------ outline

const LABEL = /^\s*([A-Za-z_][\w.]*)\s*:/;

function scanAsm(doc: Text, fromLine: number, toLine: number, out: Sym[]): void {
  for (let n = fromLine; n <= toLine; n++) {
    const line = doc.line(n);
    const m = LABEL.exec(line.text);
    if (m) out.push({ name: m[1], pos: line.from });
  }
}

/** Name node of a function definition's declarator (through pointer/reference declarators). */
function declaratorName(n: SyntaxNode): SyntaxNode | null {
  for (let c = n.firstChild; c; c = c.nextSibling) {
    if (c.name === 'FunctionDeclarator') return c.firstChild;
    if (c.name.endsWith('Declarator')) return declaratorName(c);
  }
  return null;
}

function cOutline(state: EditorState): Outline {
  const tree = syntaxTree(state);
  const doc = state.doc;
  const syms: Sym[] = [];
  for (let n = tree.topNode.firstChild; n; n = n.nextSibling) {
    if (n.name !== 'FunctionDefinition') continue;
    const id = declaratorName(n);
    if (!id) continue;
    let name = doc.sliceString(id.from, id.to);
    // avr-libc interrupt handlers: ISR(TIMER0_OVF_vect) reads better than "ISR".
    if ((name === 'ISR' || name === 'SIGNAL') && id.nextSibling?.name === 'ParameterList') name += doc.sliceString(id.nextSibling.from, id.nextSibling.to).replace(/\s+/g, '');
    syms.push({ name, pos: doc.lineAt(n.from).from });
  }
  return { kind: 'c', syms, tree };
}

function buildOutline(state: EditorState): Outline {
  const kind = state.facet(outlineKind);
  if (kind === 'c') return cOutline(state);
  const syms: Sym[] = [];
  if (kind === 'asm') scanAsm(state.doc, 1, state.doc.lines, syms);
  return { kind, syms, tree: null };
}

/** Assembly: drop symbols on changed lines, map the rest, rescan just the changed lines. */
function updateAsm(o: Outline, tr: Transaction): Outline {
  const before = tr.startState.doc;
  const doc = tr.state.doc;
  const dead: number[] = [];
  const fresh: Sym[] = [];
  tr.changes.iterChangedRanges((fA, tA, fB, tB) => {
    dead.push(before.lineAt(fA).from, before.lineAt(tA).to);
    scanAsm(doc, doc.lineAt(fB).number, doc.lineAt(tB).number, fresh);
  });
  const syms: Sym[] = [];
  outer: for (const s of o.syms) {
    for (let i = 0; i < dead.length; i += 2) if (s.pos >= dead[i] && s.pos <= dead[i + 1]) continue outer;
    syms.push({ name: s.name, pos: tr.changes.mapPos(s.pos, 1) });
  }
  if (fresh.length) {
    syms.push(...fresh);
    syms.sort((a, b) => a.pos - b.pos);
    for (let i = syms.length - 1; i > 0; i--) if (syms[i].pos === syms[i - 1].pos) syms.splice(i, 1);
  }
  return { kind: 'asm', syms, tree: null };
}

const outlineField = StateField.define<Outline>({
  create: buildOutline,
  update(o, tr) {
    const kind = tr.state.facet(outlineKind);
    if (kind !== o.kind) return buildOutline(tr.state);
    if (kind === 'c') return syntaxTree(tr.state) !== o.tree ? cOutline(tr.state) : o;
    if (kind === 'asm' && tr.docChanged) return updateAsm(o, tr);
    return o;
  },
});

// ------------------------------------------------------------------ sections

const COMMENT_LINE = /^\s*(;|\/\/|\/\*|\*)/;

/**
 * First line of each symbol's section: its definition line plus the comment lines directly
 * above it. Blank text before the first symbol joins the first section.
 */
function sectionStarts(doc: Text, syms: Sym[]): number[] {
  const starts: number[] = [];
  let floor = -1;
  for (const s of syms) {
    let line = doc.lineAt(s.pos);
    let start = line.from;
    while (line.number > 1) {
      const prev = doc.line(line.number - 1);
      if (prev.from <= floor || !COMMENT_LINE.test(prev.text)) break;
      start = prev.from;
      line = prev;
    }
    starts.push(start);
    floor = s.pos;
  }
  if (starts.length && starts[0] > 0 && !/\S/.test(doc.sliceString(0, starts[0]))) starts[0] = 0;
  return starts;
}

function sectionRegion(starts: number[], index: number): Region {
  if (!starts.length) return { from: 0, to: null };
  return index < 0 && starts[0] > 0 ? { from: 0, to: starts[0] } : { from: starts[index], to: starts[index + 1] ?? null };
}

/** Index of the section containing `pos` (-1 = text before the first symbol). */
function sectionIndex(starts: number[], pos: number): number {
  let lo = 0;
  let hi = starts.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (starts[mid] <= pos) lo = mid + 1;
    else hi = mid;
  }
  return lo - 1;
}

function sectionAt(state: EditorState, pos: number): Region {
  const starts = sectionStarts(state.doc, state.field(outlineField).syms);
  return sectionRegion(starts, sectionIndex(starts, pos));
}

/** Last position the cursor and edits may reach: the end of the section's last line. */
function regionEnd(r: Region, doc: Text): number {
  return r.to === null ? doc.length : r.to - 1;
}

// ------------------------------------------------------------------ state

const setRegion = StateEffect.define<Region | null>();

const regionField = StateField.define<Region | null>({
  create: () => null,
  update(r, tr) {
    for (const e of tr.effects) if (e.is(setRegion)) return e.value;
    if (!r || !tr.docChanged) return r;
    // Text inserted at either boundary belongs to the neighbours (edits never reach them).
    return { from: tr.changes.mapPos(r.from, -1), to: r.to === null ? null : tr.changes.mapPos(r.to, -1) };
  },
});

class HiddenLines extends WidgetType {
  constructor(readonly text: string) {
    super();
  }
  override eq(o: HiddenLines): boolean {
    return o.text === this.text;
  }
  override toDOM(): HTMLElement {
    const d = document.createElement('div');
    d.className = 'cm-symview-hidden';
    d.textContent = this.text;
    return d;
  }
}

const hiddenDecorations = EditorView.decorations.compute([regionField, 'doc'], (state): DecorationSet => {
  const r = state.field(regionField);
  if (!r) return Decoration.none;
  const doc = state.doc;
  const ranges = [];
  if (r.from > 1) {
    const last = doc.lineAt(r.from - 1).number;
    ranges.push(Decoration.replace({ block: true, widget: new HiddenLines(`⋯ lines 1–${last} (other symbols)`) }).range(0, r.from - 1));
  }
  if (r.to !== null && r.to < doc.length) {
    const first = doc.lineAt(r.to).number;
    ranges.push(Decoration.replace({ block: true, widget: new HiddenLines(`⋯ lines ${first}–${doc.lines} (other symbols)`) }).range(r.to, doc.length));
  }
  return Decoration.set(ranges);
});

/** Typing, deleting and drag-moving only change the shown section (Replace All still covers the file). */
const keepEditsInside = EditorState.changeFilter.of((tr) => {
  const r = tr.startState.field(regionField);
  if (!r || !(tr.isUserEvent('input') || tr.isUserEvent('delete') || tr.isUserEvent('move')) || tr.isUserEvent('input.replace.all')) return true;
  const end = regionEnd(r, tr.startState.doc);
  let inside = true;
  tr.changes.iterChangedRanges((a, b) => {
    if (a < r.from || b > end) inside = false;
  });
  return inside;
});

/** Cursor moves are clamped to the section; any other jump shows the section it lands in. */
const followSelection = EditorState.transactionFilter.of((tr) => {
  if (!tr.selection && !tr.docChanged) return tr;
  const state = tr.state;
  const r = state.field(regionField);
  if (!r) return tr;
  const end = regionEnd(r, state.doc);
  const sel = state.selection;
  if (sel.ranges.every((x) => x.from >= r.from && x.to <= end)) return tr;
  if (tr.isUserEvent('select') && !tr.isUserEvent('select.search')) {
    const clamp = (p: number) => Math.min(Math.max(p, r.from), end);
    return [tr, { selection: EditorSelection.create(sel.ranges.map((x) => EditorSelection.range(clamp(x.anchor), clamp(x.head))), sel.mainIndex), sequential: true }];
  }
  return [tr, { effects: setRegion.of(sectionAt(state, sel.main.head)), sequential: true }];
});

const symViewTheme = EditorView.baseTheme({
  '.cm-symview-hidden': {
    color: '#7a8698', fontStyle: 'italic', fontFamily: 'var(--font-ui)', fontSize: '11px', padding: '1px 8px',
    background: '#f4f6f9', borderTop: '1px dashed #d3dbe6', borderBottom: '1px dashed #d3dbe6',
  },
});

/** Editor extension: outline tracking plus the (initially off) section view. */
export function symbolView(): Extension {
  return [outlineField, regionField, hiddenDecorations, keepEditsInside, followSelection, symViewTheme];
}

// ------------------------------------------------------------------ commands

/** Turns the section view on (showing the cursor's symbol) or off for the view's document. */
export function setSymbolView(view: EditorView, on: boolean): void {
  const effect = symbolViewEffect(view.state, on);
  if (effect) view.dispatch({ effects: [effect, EditorView.scrollIntoView(view.state.selection.main.head, { y: 'center' })] });
}

/** The effect that turns the view on/off (null when already in that state). */
export function symbolViewEffect(state: EditorState, on: boolean): StateEffect<Region | null> | null {
  if (on === (state.field(regionField) !== null)) return null;
  return setRegion.of(on ? sectionAt(state, state.selection.main.head) : null);
}

/** First and last shown line (null with the view off). */
export function shownLines(state: EditorState): [number, number] | null {
  const r = state.field(regionField);
  return r && [state.doc.lineAt(r.from).number, state.doc.lineAt(regionEnd(r, state.doc)).number];
}

/** Symbol names in file order. */
export function outlineNames(state: EditorState): string[] {
  return state.field(outlineField).syms.map((s) => s.name);
}

/** Effect that shows the section containing `pos` when the view is on and it is hidden. */
export function revealEffect(state: EditorState, pos: number): StateEffect<Region | null> | null {
  const r = state.field(regionField);
  if (!r || (pos >= r.from && pos <= regionEnd(r, state.doc))) return null;
  return setRegion.of(sectionAt(state, pos));
}

/** Shows symbol `index` (-1 = the text before the first symbol) with the cursor on its definition. */
export function focusSymbol(view: EditorView, index: number): void {
  const state = view.state;
  const syms = state.field(outlineField).syms;
  const region = sectionRegion(sectionStarts(state.doc, syms), index);
  const at = index >= 0 && syms[index] ? state.doc.lineAt(syms[index].pos).to : region.from;
  view.dispatch({ effects: [setRegion.of(region), EditorView.scrollIntoView(at, { y: 'start', yMargin: 40 })], selection: { anchor: at } });
  view.focus();
}

/** Inserts a new symbol after the shown one (a label, or an empty C function) and shows it. */
export function addSymbol(view: EditorView, name: string): void {
  const state = view.state;
  const doc = state.doc;
  const r = state.field(regionField) ?? sectionAt(state, state.selection.main.head);
  const ind = state.facet(indentUnit);
  const c = state.facet(outlineKind) === 'c';
  const at = r.to ?? doc.length;
  const body = c ? `void ${name}(void)\n{\n${ind}` : `${name}:\n${ind}`;
  const tail = c ? '\n}\n' : '\n';
  let pre: string;
  if (r.to !== null) pre = r.to > 0 && /\S/.test(doc.lineAt(r.to - 1).text) ? '\n' : '';
  else pre = doc.length === 0 ? '' : doc.line(doc.lines).length ? '\n\n' : doc.lines > 1 && doc.line(doc.lines - 1).length ? '\n' : '';
  const insert = pre + body + tail + (r.to !== null ? '\n' : '');
  const cursor = at + pre.length + body.length;
  view.dispatch({ changes: { from: at, insert }, selection: { anchor: cursor } });
  view.dispatch({ effects: [setRegion.of(sectionAt(view.state, cursor)), EditorView.scrollIntoView(cursor, { y: 'center' })] });
  view.focus();
}

// ------------------------------------------------------------------ sidebar store

export interface OutlineItem {
  name: string;
  line: number;
}

interface SymbolViewStore {
  kind: OutlineKind;
  items: OutlineItem[];
  /** There is text before the first symbol (listed as "top of file"). */
  top: boolean;
  /** Shown (or, with the view off, cursor) section; -1 = top of file. */
  active: number;
}

export const useSymbolOutline = create<SymbolViewStore>(() => ({ kind: 'none', items: [], top: false, active: -1 }));

let queued = false;
let pendingView: EditorView | null = null;

/** Publishes the outline to the sidebar, once per burst of transactions (O(symbols)). */
export function publishOutline(view: EditorView, now = false): void {
  pendingView = view;
  if (now) publish(view);
  else if (!queued) {
    queued = true;
    queueMicrotask(() => {
      queued = false;
      if (pendingView) publish(pendingView);
    });
  }
}

function publish(view: EditorView): void {
  const state = view.state;
  const doc = state.doc;
  const o = state.field(outlineField);
  const starts = sectionStarts(doc, o.syms);
  const r = state.field(regionField);
  const active = sectionIndex(starts, r ? r.from : state.selection.main.head);
  const top = starts.length === 0 ? doc.length > 0 : starts[0] > 0;
  const prev = useSymbolOutline.getState();
  let same = prev.kind === o.kind && prev.top === top && prev.active === active && prev.items.length === o.syms.length;
  const items: OutlineItem[] = new Array(o.syms.length);
  for (let i = 0; i < o.syms.length; i++) {
    const s = o.syms[i];
    const line = doc.lineAt(s.pos).number;
    items[i] = same && prev.items[i].name === s.name && prev.items[i].line === line ? prev.items[i] : { name: s.name, line };
    if (items[i] !== prev.items[i]) same = false;
  }
  if (!same) useSymbolOutline.setState({ kind: o.kind, items, top, active });
}

/** Sidebar contents for when no document is shown. */
export function clearOutline(): void {
  pendingView = null;
  useSymbolOutline.setState({ kind: 'none', items: [], top: false, active: -1 });
}
