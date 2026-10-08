/** Number/time formatting helpers shared by panels. */
export const hex = (v: number, digits = 2) => '0x' + v.toString(16).toUpperCase().padStart(digits, '0');
export const hexRaw = (v: number, digits = 2) => v.toString(16).toUpperCase().padStart(digits, '0');
export const bin8 = (v: number) => v.toString(2).padStart(8, '0');

export function formatHz(hz: number): string {
  if (hz >= 1e6) return `${(hz / 1e6).toFixed(hz % 1e6 === 0 ? 0 : 2)} MHz`;
  if (hz >= 1e3) return `${(hz / 1e3).toFixed(hz % 1e3 === 0 ? 0 : 1)} kHz`;
  return `${hz.toFixed(0)} Hz`;
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
