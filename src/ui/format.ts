/** Number/time formatting helpers shared by panels. */
export const hex = (v: number, digits = 2) => '0x' + v.toString(16).toUpperCase().padStart(digits, '0');
export const hexRaw = (v: number, digits = 2) => v.toString(16).toUpperCase().padStart(digits, '0');
export const bin8 = (v: number) => v.toString(2).padStart(8, '0');

export function formatHz(hz: number): string {
  if (hz >= 1e9) return `${+(hz / 1e9).toFixed(2)} GHz`;
  if (hz >= 1e6) return `${(hz / 1e6).toFixed(hz % 1e6 === 0 ? 0 : 2)} MHz`;
  if (hz >= 1e3) return `${(hz / 1e3).toFixed(hz % 1e3 === 0 ? 0 : 1)} kHz`;
  if (hz > 0 && hz < 10 && hz % 1 !== 0) return `${+hz.toFixed(2)} Hz`;
  return `${hz.toFixed(0)} Hz`;
}

/** Parses "8 MHz", "32.768kHz", "16e6", "1000" (Hz). Returns NaN when invalid. */
export function parseHz(text: string): number {
  const m = /^\s*([0-9]*\.?[0-9]+(?:e[+-]?\d+)?)\s*(g|m|k)?\s*(hz)?\s*$/i.exec(text);
  if (!m) return NaN;
  const mult = { g: 1e9, m: 1e6, k: 1e3 }[(m[2] ?? '').toLowerCase() as 'g' | 'm' | 'k'] ?? 1;
  return Number(m[1]) * mult;
}

export function formatTime(sec: number): string {
  if (sec >= 1) return `${sec.toFixed(4)} s`;
  if (sec >= 1e-3) return `${(sec * 1e3).toFixed(3)} ms`;
  return `${(sec * 1e6).toFixed(2)} µs`;
}

/** Parses "0x1F", "$1F", "0b101", "31" (returns NaN when invalid). */
export function parseNumber(text: string): number {
  const t = text.trim().toLowerCase();
  if (/^(0x|\$)[0-9a-f]+$/.test(t)) return parseInt(t.replace(/^(0x|\$)/, ''), 16);
  if (/^0b[01]+$/.test(t)) return parseInt(t.slice(2), 2);
  if (/^-?\d+$/.test(t)) return parseInt(t, 10);
  if (/^[0-9a-f]+h$/.test(t)) return parseInt(t.slice(0, -1), 16);
  return NaN;
}
