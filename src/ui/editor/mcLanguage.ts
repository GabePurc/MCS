/**
 * Machine-code (.mc) files: syntax highlighting plus live disassembly shown at the end of every
 * line, so hand-written opcodes can be checked while typing. Decoding happens in the Rust core
 * (`machineCodeHints`), debounced.
 */
import { StateEffect, StateField, type Extension } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, WidgetType, type DecorationSet, type ViewUpdate } from '@codemirror/view';
import { StreamLanguage, type StreamParser } from '@codemirror/language';
import { machineCodeHints } from '../backend/api';
import type { McHint } from '../backend/types';
import { useSettings } from '../state/settings';

const parser: StreamParser<{ bin: number }> = {
  name: 'avrmc',
  startState: () => ({ bin: 0 }),
  token(stream, state) {
    if (stream.sol()) state.bin = 0;
    if (stream.eatSpace()) return null;
    if (stream.match(';') || stream.match('//') || stream.match('#')) {
      stream.skipToEnd();
      return 'comment';
    }
    if (stream.match(/^@\S*/) || stream.match(/^\.org\b/i)) return 'keyword';
    if (stream.match(/^(?:0x)?[0-9a-fA-F]+:/)) return 'labelName';
    const bin = stream.match(/^0b[01_]+/i) as RegExpMatchArray | null;
    if (bin) {
      state.bin = 16 - bin[0].slice(2).replace(/_/g, '').length;
      return 'string';
    }
    if (state.bin > 0) {
      const g = stream.match(/^[01_]+/) as RegExpMatchArray | null;
      if (g) {
        state.bin -= g[0].replace(/_/g, '').length;
        return 'string';
      }
    }
    if (stream.match(/^(?:0x|\$)?[0-9a-fA-F]+\b/)) return 'number';
    stream.next();
    return 'invalid';
  },
};

export const avrMachineCode = StreamLanguage.define(parser);

class HintWidget extends WidgetType {
  constructor(readonly h: McHint) {
    super();
  }
  override eq(o: HintWidget) {
    return o.h.text === this.h.text && o.h.address === this.h.address && o.h.valid === this.h.valid;
  }
  override toDOM() {
    const s = document.createElement('span');
    s.className = `cm-mc-hint${this.h.valid ? '' : ' invalid'}`;
    s.textContent = `${this.h.address.toString(16).toUpperCase().padStart(4, '0')}  ${this.h.text}`;
    return s;
  }
  override ignoreEvent() {
    return true;
  }
}

const setHints = StateEffect.define<McHint[]>();

const hintField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    for (const e of tr.effects) {
      if (e.is(setHints)) {
        const doc = tr.state.doc;
        return Decoration.set(
          e.value
            .filter((h) => h.line >= 1 && h.line <= doc.lines)
            // Lines whose comment already spells out the instruction need no hint.
            .filter((h) => !h.valid || !doc.line(h.line).text.toLowerCase().includes(h.text.toLowerCase()))
            .map((h) => Decoration.widget({ widget: new HintWidget(h), side: 1 }).range(doc.line(h.line).to)),
          true,
        );
      }
    }
    return tr.docChanged ? deco.map(tr.changes) : deco;
  },
  provide: (f) => EditorView.decorations.from(f),
});

const hintPlugin = ViewPlugin.fromClass(
  class {
    timer = 0;
    seq = 0;
    constructor(readonly view: EditorView) {
      this.schedule(0);
    }
    update(u: ViewUpdate) {
      if (u.docChanged) this.schedule(180);
    }
    schedule(ms: number) {
      clearTimeout(this.timer);
      this.timer = window.setTimeout(() => {
        const seq = ++this.seq;
        machineCodeHints(this.view.state.doc.toString(), useSettings.getState().deviceId)
          .then((r) => seq === this.seq && this.view.dispatch({ effects: setHints.of(r.hints) }))
          .catch(() => {});
      }, ms);
    }
    destroy() {
      clearTimeout(this.timer);
      this.seq++;
    }
  },
);

/** Language + live disassembly hints for machine-code documents. */
export function machineCodeSupport(): Extension {
  return [avrMachineCode, hintField, hintPlugin];
}
