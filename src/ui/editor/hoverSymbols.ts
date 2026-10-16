/** Pure helpers behind the editor hover: choosing the source symbol under the cursor. */
import type { SourceSymbol } from '../backend/types';

/**
 * The symbol named `word` visible on `line` (1-based): the innermost enclosing scope wins, then
 * file-scope symbols (first definition). Symbols whose scope does not contain `line` never match.
 */
export function pickSymbol(syms: readonly SourceSymbol[], word: string, line: number, caseInsensitive: boolean): SourceSymbol | undefined {
  const key = caseInsensitive ? word.toLowerCase() : word;
  let best: SourceSymbol | undefined;
  let fileScope: SourceSymbol | undefined;
  for (const s of syms) {
    if ((caseInsensitive ? s.name.toLowerCase() : s.name) !== key) continue;
    if (!s.scope) {
      fileScope ??= s;
    } else if (line >= s.scope[0] && line <= s.scope[1]) {
      if (!best || s.scope[1] - s.scope[0] <= best.scope![1] - best.scope![0]) best = s;
    }
  }
  return best ?? fileScope;
}

/** "8 (0x08)" style number for constants. */
export function formatValue(v: number): string {
  return v >= 0 ? `${v} (0x${v.toString(16).toUpperCase().padStart(v > 0xff ? 4 : 2, '0')})` : String(v);
}

/** Lines of the tooltip: signature, kind/line/value, doc comment. */
export function symbolText(s: SourceSymbol): { detail: string; meta: string; doc: string } {
  let meta = s.kind;
  if (s.line > 0) meta += ` — line ${s.line}`;
  // Values of register aliases are register numbers and of SRAM variables addresses: shown by `detail`/live data.
  if (s.value !== null && s.kind !== 'register alias' && !(s.kind === 'variable' && s.detail.includes('SRAM')) && !(s.kind === 'label')) meta += ` = ${formatValue(s.value)}`;
  return { detail: s.detail, meta, doc: s.doc };
}

/** Remembers the symbols of the last source text so repeated hovers do not call the backend again. */
export function createSymbolCache(fetch: (source: string, language: string, deviceId: string) => Promise<SourceSymbol[]>) {
  let key: string | null = null;
  let text = '';
  let promise: Promise<SourceSymbol[]> = Promise.resolve([]);
  return (source: string, language: string, deviceId: string): Promise<SourceSymbol[]> => {
    const k = `${language}\0${deviceId}`;
    if (key === k && text === source) return promise;
    key = k;
    text = source;
    promise = fetch(source, language, deviceId).catch(() => {
      if (text === source) key = null; // do not cache failures
      return [];
    });
    return promise;
  };
}
