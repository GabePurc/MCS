import { describe, expect, it } from 'vitest';
import { EditorSelection, EditorState } from '@codemirror/state';
import { cpp } from '@codemirror/lang-cpp';
import { ensureSyntaxTree } from '@codemirror/language';
import { outlineKind, outlineNames, revealEffect, shownLines, symbolView, symbolViewEffect } from './symbolView';

const ASM = [
  '.include "tn10def.inc"', // 1
  '', // 2
  '; entry point', // 3
  'main:', // 4
  '        ldi r16, 1', // 5
  '', // 6
  'loop:  rjmp loop', // 7
  '', // 8
  '; waits', // 9
  '; a while', // 10
  'delay:', // 11
  '        ret', // 12
].join('\n');

function asmState(cursorLine: number): EditorState {
  let st = EditorState.create({ doc: ASM, extensions: [symbolView(), outlineKind.of('asm')] });
  st = st.update({ selection: { anchor: st.doc.line(cursorLine).from } }).state;
  return st.update({ effects: symbolViewEffect(st, true)! }).state;
}

describe('symbol view', () => {
  it('lists assembly labels and shows the cursor section with its leading comments', () => {
    const st = asmState(12);
    expect(outlineNames(st)).toEqual(['main', 'loop', 'delay']);
    expect(shownLines(st)).toEqual([9, 12]);
    expect(shownLines(asmState(5))).toEqual([3, 6]);
    expect(shownLines(asmState(1))).toEqual([1, 2]);
  });

  it('keeps the outline in step with edits', () => {
    let st = asmState(5);
    st = st.update({ changes: { from: st.doc.line(5).to, insert: '\ninner:' }, userEvent: 'input' }).state;
    expect(outlineNames(st)).toEqual(['main', 'inner', 'loop', 'delay']);
    st = st.update({ changes: { from: st.doc.line(4).from, to: st.doc.line(4).from + 4, insert: 'start' }, userEvent: 'input' }).state;
    expect(outlineNames(st)).toEqual(['start', 'inner', 'loop', 'delay']);
    // The shown section grew with the typed line.
    expect(shownLines(st)).toEqual([3, 7]);
  });

  it('blocks typing outside the shown section and clamps cursor moves', () => {
    const st = asmState(5);
    const outside = st.update({ changes: { from: st.doc.line(8).from, insert: 'x' }, userEvent: 'input.type' }).state;
    expect(outside.doc.toString()).toBe(ASM);
    const across = st.update({ changes: { from: st.doc.line(3).from - 1, to: st.doc.line(3).from }, userEvent: 'delete.backward' }).state;
    expect(across.doc.toString()).toBe(ASM);
    const moved = st.update({ selection: { anchor: st.doc.line(11).from }, userEvent: 'select' }).state;
    expect(moved.selection.main.head).toBe(st.doc.line(6).to);
    expect(shownLines(moved)).toEqual([3, 6]);
  });

  it('follows programmatic jumps and the debugger into other sections', () => {
    const st = asmState(5);
    const jumped = st.update({ selection: EditorSelection.cursor(st.doc.line(12).from) }).state;
    expect(shownLines(jumped)).toEqual([9, 12]);
    expect(revealEffect(st, st.doc.line(5).from)).toBeNull();
    const revealed = st.update({ effects: revealEffect(st, st.doc.line(7).from)! }).state;
    expect(shownLines(revealed)).toEqual([7, 8]);
  });

  it('lists top-level C functions, ISRs by vector', () => {
    const src = '#include <avr/io.h>\nint x;\n// toggles\nstatic void toggle(void) { PORTB ^= 1; }\nchar *name(int a)\n{\n  return 0;\n}\nISR(TIM0_OVF_vect) { }\nint main(void) { for (;;) {} }\n';
    const st = EditorState.create({ doc: src, extensions: [cpp(), symbolView(), outlineKind.of('c')] });
    ensureSyntaxTree(st, st.doc.length, 1e9);
    const st2 = st.update({}).state;
    expect(outlineNames(st2)).toEqual(['toggle', 'name', 'ISR(TIM0_OVF_vect)', 'main']);
    const on = st2.update({ selection: { anchor: st2.doc.line(4).from } }).state;
    expect(shownLines(on.update({ effects: symbolViewEffect(on, true)! }).state)).toEqual([3, 4]);
  });
});
