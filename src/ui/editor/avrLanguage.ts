/**
 * AVR assembly (avrasm2 dialect) syntax highlighting, completion and hover info for
 * CodeMirror, plus a Visual Studio 2010 flavoured highlight style.
 */
import { StreamLanguage, HighlightStyle, type StreamParser } from '@codemirror/language';
import { tags as t } from '@lezer/highlight';
import type { CompletionContext, CompletionResult } from '@codemirror/autocomplete';

export const MNEMONICS = (
  'add adc adiw sub subi sbc sbci sbiw and andi or ori eor com neg sbr cbr inc dec tst clr ser mul muls mulsu fmul fmuls fmulsu des ' +
  'rjmp ijmp eijmp jmp rcall icall eicall call ret reti cpse cp cpc cpi sbrc sbrs sbic sbis brbs brbc breq brne brcs brcc brsh brlo brmi brpl ' +
  'brge brlt brhs brhc brts brtc brvs brvc brie brid mov movw ldi lds ld ldd sts st std lpm elpm spm in out push pop xch las lac lat ' +
  'lsl lsr rol ror asr swap bset bclr sbi cbi bst bld sec clc sen cln sez clz sei cli ses cls sev clv set clt seh clh break nop sleep wdr'
).split(' ');

const MNEMONIC_SET = new Set(MNEMONICS);
export const DIRECTIVES = [
  'include', 'device', 'def', 'undef', 'equ', 'set', 'org', 'cseg', 'dseg', 'eseg', 'byte', 'db', 'dw', 'dd', 'dq', 'macro', 'endm', 'endmacro',
  'if', 'elif', 'elseif', 'else', 'endif', 'ifdef', 'ifndef', 'error', 'warning', 'message', 'list', 'nolist', 'listmac', 'exit',
];
const FUNCTIONS = new Set(['low', 'high', 'byte1', 'byte2', 'byte3', 'byte4', 'lwrd', 'hwrd', 'page', 'exp2', 'log2', 'abs', 'lo8', 'hi8', 'pm']);

/** Register/bit names of the current device (updated by the editor host). */
export const deviceWords = { registers: new Set<string>(), bits: new Set<string>() };

interface State {
  inComment: boolean;
}

const parser: StreamParser<State> = {
  name: 'avrasm',
  startState: () => ({ inComment: false }),
  token(stream, state) {
    if (state.inComment) {
      if (stream.skipTo('*/')) {
        stream.match('*/');
        state.inComment = false;
      } else stream.skipToEnd();
      return 'comment';
    }
    if (stream.eatSpace()) return null;
    if (stream.match('/*')) {
      state.inComment = true;
      return 'comment';
    }
    if (stream.match(';') || stream.match('//')) {
      stream.skipToEnd();
      return 'comment';
    }
    if (stream.match(/^"(?:[^"\\]|\\.)*"?/)) return 'string';
    if (stream.match(/^'(?:[^'\\]|\\.)'?/)) return 'string';
    if (stream.match(/^\.[A-Za-z_]+/)) return 'keyword';
    if (stream.match(/^#[A-Za-z_]+/)) return 'meta';
    if (stream.match(/^(?:0x[0-9a-fA-F]+|\$[0-9a-fA-F]+|0b[01]+|\d+)/)) return 'number';
    if (stream.match(/^@\d/)) return 'variableName.special';
    const word = stream.match(/^[A-Za-z_][A-Za-z0-9_]*/) as RegExpMatchArray | null;
    if (word) {
      const w = word[0];
      const lw = w.toLowerCase();
      if (stream.peek() === ':') return 'labelName';
      if (/^r([0-9]|[12][0-9]|3[01])$/i.test(w) || /^[xyz][hl]?$/i.test(w)) return 'variableName.special';
      if (MNEMONIC_SET.has(lw)) return 'operatorKeyword';
      if (FUNCTIONS.has(lw)) return 'function';
      const uw = w.toUpperCase();
      if (deviceWords.registers.has(uw)) return 'typeName';
      if (deviceWords.bits.has(uw)) return 'propertyName';
      return 'variableName';
    }
    stream.next();
    return 'operator';
  },
  languageData: { commentTokens: { line: ';', block: { open: '/*', close: '*/' } } },
};

export const avrAsm = StreamLanguage.define(parser);

/** Visual Studio 2010 style colours. */
export const vsHighlight = HighlightStyle.define([
  { tag: t.comment, color: '#008000' },
  { tag: t.keyword, color: '#0000ff' },
  { tag: t.controlKeyword, color: '#0000ff' },
  { tag: t.operatorKeyword, color: '#00008b', fontWeight: '600' },
  { tag: t.definitionKeyword, color: '#0000ff' },
  { tag: t.modifier, color: '#0000ff' },
  { tag: [t.string, t.character], color: '#a31515' },
  { tag: t.number, color: '#000000' },
  { tag: t.meta, color: '#808080' },
  { tag: t.processingInstruction, color: '#808080' },
  { tag: t.labelName, color: '#8b008b', fontWeight: '600' },
  { tag: t.special(t.variableName), color: '#a0522d' },
  { tag: t.typeName, color: '#2b91af' },
  { tag: t.propertyName, color: '#6f008a' },
  { tag: t.function(t.variableName), color: '#795e26' },
  { tag: t.macroName, color: '#6f008a' },
  { tag: t.operator, color: '#000000' },
]);

/** Completion of mnemonics, directives, registers and device names in assembly sources. */
export function asmCompletions(ctx: CompletionContext): CompletionResult | null {
  const word = ctx.matchBefore(/\.?[A-Za-z_][A-Za-z0-9_]*/);
  if (!word || (word.from === word.to && !ctx.explicit)) return null;
  if (word.text.startsWith('.')) {
    return { from: word.from, options: DIRECTIVES.map((d) => ({ label: `.${d}`, type: 'keyword' })) };
  }
  const line = ctx.state.doc.lineAt(ctx.pos);
  const before = line.text.slice(0, word.from - line.from);
  const atMnemonic = /^\s*([A-Za-z_]\w*:\s*)?$/.test(before);
  const options = atMnemonic
    ? MNEMONICS.map((m) => ({ label: m, type: 'keyword' }))
    : [
        ...Array.from({ length: 32 }, (_, i) => ({ label: `r${i}`, type: 'variable' })),
        ...[...deviceWords.registers].map((r) => ({ label: r, type: 'type', detail: 'I/O register' })),
        ...[...deviceWords.bits].map((b) => ({ label: b, type: 'property', detail: 'bit' })),
        ...['low', 'high', 'lo8', 'hi8'].map((f) => ({ label: f, type: 'function' })),
      ];
  return { from: word.from, options, validFor: /^[A-Za-z0-9_]*$/ };
}
