/**
 * Glossy Windows 7-era 16x16 icons drawn as inline SVG. Gradients live in a single shared
 * <defs> block (IconDefs) so each icon instance stays tiny.
 */
import type { JSX } from 'react';

export function IconDefs(): JSX.Element {
  const lin = (id: string, stops: [number, string][], x2 = 0, y2 = 1) => (
    <linearGradient id={id} x1="0" y1="0" x2={x2} y2={y2}>
      {stops.map(([o, c]) => <stop key={o} offset={o} stopColor={c} />)}
    </linearGradient>
  );
  const rad = (id: string, stops: [number, string][]) => (
    <radialGradient id={id} cx="0.35" cy="0.3" r="0.75">
      {stops.map(([o, c]) => <stop key={o} offset={o} stopColor={c} />)}
    </radialGradient>
  );
  return (
    <svg width="0" height="0" style={{ position: 'absolute' }} aria-hidden>
      <defs>
        {lin('ig-green', [[0, '#b9f3a5'], [0.5, '#3fbf2a'], [1, '#1d7d15']])}
        {lin('ig-blue', [[0, '#bfe0ff'], [0.5, '#3d8fe0'], [1, '#1c4f9c']])}
        {lin('ig-red', [[0, '#ffb3a6'], [0.5, '#e0432a'], [1, '#9b1d0c']])}
        {lin('ig-yellow', [[0, '#fff6c2'], [0.5, '#ffd23b'], [1, '#d38c00']])}
        {lin('ig-folder', [[0, '#fff1b5'], [0.6, '#f8cc55'], [1, '#d99a1e']])}
        {lin('ig-paper', [[0, '#ffffff'], [1, '#e1e8f1']], 1, 1)}
        {lin('ig-chip', [[0, '#5b6675'], [0.5, '#2f3843'], [1, '#1c232b']])}
        {lin('ig-steel', [[0, '#f4f6f9'], [0.5, '#c4cdd8'], [1, '#8d99a8']])}
        {lin('ig-floppy', [[0, '#7fb0e8'], [1, '#2a5ea8']])}
        {lin('ig-purple', [[0, '#e3c8ff'], [0.5, '#9a5ad9'], [1, '#5b2694']])}
        {lin('ig-teal', [[0, '#bff3f0'], [0.5, '#2bb3aa'], [1, '#13706a']])}
        {rad('ig-ball-red', [[0, '#ffd1c7'], [0.45, '#e2412a'], [1, '#8f1606']])}
        {rad('ig-ball-green', [[0, '#e3ffd9'], [0.45, '#3ec22b'], [1, '#166e0b']])}
        {rad('ig-ball-dark', [[0, '#9fb0c2'], [0.45, '#4d5c6c'], [1, '#232c35']])}
        {rad('ig-ball-amber', [[0, '#fff5c9'], [0.45, '#f2b51c'], [1, '#9a650a']])}
      </defs>
    </svg>
  );
}

type P = { size?: number; className?: string };
const S = ({ size = 16, className, children }: P & { children: React.ReactNode }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" className={className} aria-hidden>
    {children}
  </svg>
);

export const Icons = {
  App: (p: P) => (
    <S {...p}>
      <rect x="3" y="3" width="10" height="10" rx="1.5" fill="url(#ig-chip)" stroke="#11161c" strokeWidth="0.6" />
      {[4.5, 7, 9.5].map((y) => <g key={y}><rect x="0.8" y={y} width="2.4" height="1.2" fill="#d9b44a" /><rect x="12.8" y={y} width="2.4" height="1.2" fill="#d9b44a" /></g>)}
      <rect x="4.5" y="4.5" width="7" height="7" rx="1" fill="none" stroke="#6cb2f5" strokeWidth="0.7" opacity="0.8" />
      <circle cx="5.3" cy="5.3" r="0.7" fill="#cfd8e2" />
    </S>
  ),
  NewFile: (p: P) => (
    <S {...p}>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="url(#ig-paper)" stroke="#7a8ea8" />
      <path d="M9.5 1.5v3h3" fill="#dfe6ef" stroke="#7a8ea8" />
      <path d="M11.5 9.5l.8 1.6 1.7.3-1.2 1.2.3 1.7-1.6-.8-1.6.8.3-1.7-1.2-1.2 1.7-.3z" fill="url(#ig-yellow)" stroke="#b07a00" strokeWidth="0.5" />
    </S>
  ),
  Open: (p: P) => (
    <S {...p}>
      <path d="M1.5 3.5h4l1 1.5h6v8.5h-11z" fill="#e8b84a" stroke="#a87410" />
      <path d="M2.5 13.5l2-6h10l-2 6z" fill="url(#ig-folder)" stroke="#a87410" />
    </S>
  ),
  Save: (p: P) => (
    <S {...p}>
      <path d="M1.5 1.5h11l2 2v11h-13z" fill="url(#ig-floppy)" stroke="#1f437a" />
      <rect x="4" y="2" width="7" height="4.5" fill="#f4f7fb" stroke="#6a87ad" strokeWidth="0.6" />
      <rect x="8.5" y="2.6" width="1.5" height="3.2" fill="#2a5ea8" />
      <rect x="3.5" y="9" width="9" height="5" rx="0.5" fill="#e9eef5" stroke="#6a87ad" strokeWidth="0.6" />
    </S>
  ),
  Build: (p: P) => (
    <S {...p}>
      <path d="M8 1.2l1.3.2.4 1.6 1.2.6 1.4-.9 1 1-.9 1.4.6 1.2 1.6.4.2 1.3-.2 1.3-1.6.4-.6 1.2.9 1.4-1 1-1.4-.9-1.2.6-.4 1.6-1.3.2-1.3-.2-.4-1.6-1.2-.6-1.4.9-1-1 .9-1.4-.6-1.2-1.6-.4L1.2 8l.2-1.3L3 6.3l.6-1.2-.9-1.4 1-1 1.4.9 1.2-.6.4-1.6z" fill="url(#ig-steel)" stroke="#5c6878" strokeWidth="0.7" />
      <circle cx="8" cy="8" r="2.4" fill="#fff" stroke="#5c6878" strokeWidth="0.7" />
    </S>
  ),
  Run: (p: P) => (
    <S {...p}>
      <path d="M3.5 1.8l10 6.2-10 6.2z" fill="url(#ig-green)" stroke="#155d0f" strokeLinejoin="round" />
      <path d="M4.5 3.6l6.3 3.8-6.3.4z" fill="#fff" opacity="0.45" />
    </S>
  ),
  Pause: (p: P) => (
    <S {...p}>
      <rect x="3" y="2.5" width="3.6" height="11" rx="0.8" fill="url(#ig-blue)" stroke="#183f7a" />
      <rect x="9.4" y="2.5" width="3.6" height="11" rx="0.8" fill="url(#ig-blue)" stroke="#183f7a" />
    </S>
  ),
  Stop: (p: P) => (
    <S {...p}>
      <rect x="2.5" y="2.5" width="11" height="11" rx="1.5" fill="url(#ig-red)" stroke="#7a1406" />
      <rect x="3.6" y="3.4" width="8.8" height="4" rx="1" fill="#fff" opacity="0.35" />
    </S>
  ),
  Reset: (p: P) => (
    <S {...p}>
      <path d="M12.6 5.2A5.5 5.5 0 1 0 13.5 9" fill="none" stroke="#1c4f9c" strokeWidth="2.6" strokeLinecap="round" />
      <path d="M12.6 5.2A5.5 5.5 0 1 0 13.5 9" fill="none" stroke="url(#ig-blue)" strokeWidth="1.6" strokeLinecap="round" />
      <path d="M14.6 1.8l-.2 5.2-5-1.6z" fill="url(#ig-blue)" stroke="#183f7a" strokeWidth="0.7" strokeLinejoin="round" />
    </S>
  ),
  StepInto: (p: P) => (
    <S {...p}>
      <path d="M4 2.5h4.5a3 3 0 0 1 3 3v3.5" fill="none" stroke="#1c4f9c" strokeWidth="1.8" />
      <path d="M8.3 8.2h6.4L11.5 12z" fill="url(#ig-blue)" stroke="#183f7a" strokeWidth="0.7" />
      <circle cx="11.5" cy="14" r="1.6" fill="url(#ig-ball-dark)" />
      <rect x="1" y="1.6" width="3" height="1.8" fill="#7a8ea8" />
    </S>
  ),
  StepOver: (p: P) => (
    <S {...p}>
      <path d="M2.5 9.5a5.5 5.5 0 0 1 10-3" fill="none" stroke="#1c4f9c" strokeWidth="1.8" />
      <path d="M14.8 3.8l-.6 5-4.4-2.4z" fill="url(#ig-blue)" stroke="#183f7a" strokeWidth="0.7" />
      <circle cx="8" cy="13.6" r="1.6" fill="url(#ig-ball-dark)" />
    </S>
  ),
  StepOut: (p: P) => (
    <S {...p}>
      <path d="M8 13V4.5" stroke="#1c4f9c" strokeWidth="1.8" />
      <path d="M4.3 6.2L8 1.6l3.7 4.6z" fill="url(#ig-blue)" stroke="#183f7a" strokeWidth="0.7" />
      <circle cx="3.2" cy="13.6" r="1.6" fill="url(#ig-ball-dark)" />
      <rect x="9.5" y="12.6" width="5" height="2" fill="#7a8ea8" />
    </S>
  ),
  RunToCursor: (p: P) => (
    <S {...p}>
      <path d="M1.5 8h8" stroke="#1d7d15" strokeWidth="1.8" />
      <path d="M8.5 4.6L12.5 8l-4 3.4z" fill="url(#ig-green)" stroke="#155d0f" strokeWidth="0.7" />
      <path d="M14 3v10M12.8 3h2.4M12.8 13h2.4" stroke="#333" strokeWidth="1" />
    </S>
  ),
  Breakpoint: (p: P) => (
    <S {...p}>
      <circle cx="8" cy="8" r="5.6" fill="url(#ig-ball-red)" stroke="#7a1406" strokeWidth="0.8" />
    </S>
  ),
  ClearBreakpoints: (p: P) => (
    <S {...p}>
      <circle cx="7" cy="7" r="5" fill="url(#ig-ball-red)" stroke="#7a1406" strokeWidth="0.8" />
      <path d="M9.5 9.5l5 5M14.5 9.5l-5 5" stroke="#333" strokeWidth="1.6" />
    </S>
  ),
  Import: (p: P) => (
    <S {...p}>
      <rect x="4" y="5" width="8" height="8" rx="1" fill="url(#ig-chip)" stroke="#11161c" strokeWidth="0.6" />
      {[6.5, 9, 11.2].map((y) => <g key={y}><rect x="2" y={y} width="2" height="1" fill="#d9b44a" /><rect x="12" y={y} width="2" height="1" fill="#d9b44a" /></g>)}
      <path d="M8 0.6v4.6M5.8 3.2L8 5.6l2.2-2.4" stroke="#1d7d15" strokeWidth="1.7" fill="none" />
    </S>
  ),
  Export: (p: P) => (
    <S {...p}>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="url(#ig-paper)" stroke="#7a8ea8" />
      <path d="M5.5 9.5h4M7.5 7.3l2.3 2.2-2.3 2.2" stroke="#1d7d15" strokeWidth="1.5" fill="none" />
    </S>
  ),
  Cpu: (p: P) => (
    <S {...p}>
      <rect x="3.5" y="3.5" width="9" height="9" rx="1" fill="url(#ig-chip)" stroke="#11161c" strokeWidth="0.6" />
      {[5, 8, 11].map((v) => <g key={v} stroke="#9aa6b3" strokeWidth="1"><path d={`M${v} 1v2.5M${v} 12.5V15M1 ${v}h2.5M12.5 ${v}H15`} /></g>)}
      <text x="8" y="10.2" fontSize="5.5" textAnchor="middle" fill="#7fd0ff" fontFamily="Arial" fontWeight="bold">R</text>
    </S>
  ),
  Io: (p: P) => (
    <S {...p}>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" fill="url(#ig-paper)" stroke="#7a8ea8" />
      {[0, 1, 2, 3].map((i) => <rect key={i} x={3 + i * 2.6} y="5" width="2" height="2" fill={i % 2 ? '#fff' : '#1c4f9c'} stroke="#1c4f9c" strokeWidth="0.5" />)}
      {[0, 1, 2, 3].map((i) => <rect key={i} x={3 + i * 2.6} y="9" width="2" height="2" fill={i === 1 || i === 2 ? '#1c4f9c' : '#fff'} stroke="#1c4f9c" strokeWidth="0.5" />)}
    </S>
  ),
  Memory: (p: P) => (
    <S {...p}>
      <rect x="1" y="4" width="14" height="7" rx="0.5" fill="url(#ig-teal)" stroke="#0e524d" strokeWidth="0.7" />
      {[2.5, 5.7, 8.9, 12.1].map((x) => <rect key={x} x={x} y="5.5" width="2" height="3.5" fill="#1b2b2a" />)}
      {[2, 4, 6, 8, 10, 12, 14].map((x) => <rect key={x} x={x - 0.5} y="11" width="1" height="2.2" fill="#d9b44a" />)}
    </S>
  ),
  Disasm: (p: P) => (
    <S {...p}>
      <path d="M2.5 1.5h11v13h-11z" fill="url(#ig-paper)" stroke="#7a8ea8" />
      <path d="M4 4h3M8 4h4M4 6.5h2.5M8 6.5h3M4 9h3M8 9h4.5M4 11.5h2M8 11.5h3" stroke="#1c4f9c" strokeWidth="1" />
    </S>
  ),
  Pins: (p: P) => (
    <S {...p}>
      <rect x="4" y="3" width="8" height="10" rx="1" fill="url(#ig-chip)" stroke="#11161c" strokeWidth="0.6" />
      {[4.5, 7.5, 10.5].map((y) => <g key={y}><rect x="1" y={y} width="3" height="1.4" fill="#d9b44a" /><rect x="12" y={y} width="3" height="1.4" fill="#d9b44a" /></g>)}
      <circle cx="13.5" cy="2.5" r="2" fill="url(#ig-ball-green)" />
    </S>
  ),
  Wave: (p: P) => (
    <S {...p}>
      <rect x="0.5" y="1.5" width="15" height="13" rx="1" fill="#10202c" stroke="#3a4b5c" />
      <path d="M1.5 6.5h2v-3h3v3h2v-3h3v3h3" fill="none" stroke="#5df07b" strokeWidth="1" />
      <path d="M1.5 12.5h4v-3h5v3h4" fill="none" stroke="#ffd23b" strokeWidth="1" />
    </S>
  ),
  Output: (p: P) => (
    <S {...p}>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" fill="url(#ig-paper)" stroke="#7a8ea8" />
      <rect x="1.5" y="2.5" width="13" height="2.5" fill="url(#ig-blue)" />
      <path d="M3.5 7.5h7M3.5 9.5h9M3.5 11.5h5" stroke="#5c6878" strokeWidth="1" />
    </S>
  ),
  Symbols: (p: P) => (
    <S {...p}>
      <path d="M2 4.5l6-3 6 3v7l-6 3-6-3z" fill="url(#ig-purple)" stroke="#40186c" strokeWidth="0.7" />
      <path d="M2 4.5l6 3 6-3M8 7.5v7" stroke="#e6d4fb" strokeWidth="0.8" fill="none" />
    </S>
  ),
  CallStack: (p: P) => (
    <S {...p}>
      {[0, 1, 2].map((i) => <rect key={i} x={2 + i} y={10 - i * 3.5} width={12 - i * 2} height="3" rx="0.6" fill={i === 2 ? 'url(#ig-yellow)' : 'url(#ig-blue)'} stroke={i === 2 ? '#a87410' : '#183f7a'} strokeWidth="0.6" />)}
    </S>
  ),
  List: (p: P) => (
    <S {...p}>
      {[3, 7, 11].map((y) => <g key={y}><circle cx="3" cy={y + 0.5} r="1.6" fill="url(#ig-ball-red)" /><path d={`M6 ${y + 0.5}h8`} stroke="#5c6878" strokeWidth="1.2" /></g>)}
    </S>
  ),
  Info: (p: P) => (
    <S {...p}>
      <circle cx="8" cy="8" r="6.5" fill="url(#ig-blue)" stroke="#183f7a" strokeWidth="0.8" />
      <rect x="7.1" y="6.8" width="1.8" height="5" fill="#fff" />
      <circle cx="8" cy="4.6" r="1.1" fill="#fff" />
    </S>
  ),
  Warning: (p: P) => (
    <S {...p}>
      <path d="M8 1.3L15 14H1z" fill="url(#ig-yellow)" stroke="#a87410" strokeLinejoin="round" />
      <rect x="7.2" y="5.2" width="1.6" height="5" fill="#3a2a00" />
      <circle cx="8" cy="11.8" r="0.95" fill="#3a2a00" />
    </S>
  ),
  Error: (p: P) => (
    <S {...p}>
      <circle cx="8" cy="8" r="6.5" fill="url(#ig-ball-red)" stroke="#7a1406" strokeWidth="0.8" />
      <path d="M5.4 5.4l5.2 5.2M10.6 5.4l-5.2 5.2" stroke="#fff" strokeWidth="1.7" />
    </S>
  ),
  Success: (p: P) => (
    <S {...p}>
      <circle cx="8" cy="8" r="6.5" fill="url(#ig-ball-green)" stroke="#155d0f" strokeWidth="0.8" />
      <path d="M4.8 8.2l2.2 2.2 4.2-4.6" stroke="#fff" strokeWidth="1.8" fill="none" />
    </S>
  ),
  Clear: (p: P) => (
    <S {...p}>
      <path d="M2.5 10.5l6-7 5 4.5-5.5 6.5H5z" fill="url(#ig-paper)" stroke="#7a8ea8" />
      <path d="M2.5 10.5l3.4-4 5 4.5-3 3.5H5z" fill="url(#ig-red)" stroke="#7a1406" strokeWidth="0.7" />
    </S>
  ),
  Lock: (p: P) => (
    <S {...p}>
      <path d="M5 7V5a3 3 0 0 1 6 0v2" fill="none" stroke="#7a8ea8" strokeWidth="1.6" />
      <rect x="3" y="7" width="10" height="7" rx="1" fill="url(#ig-yellow)" stroke="#a87410" />
    </S>
  ),
  Settings: (p: P) => (
    <S {...p}>
      <path d="M10.6 2.2a3.4 3.4 0 0 0-4.2 4.3L1.8 11.1a1.5 1.5 0 0 0 2.1 2.1l4.6-4.6a3.4 3.4 0 0 0 4.3-4.2l-2 2-1.9-.4-.4-1.9z" fill="url(#ig-steel)" stroke="#5c6878" strokeWidth="0.8" />
    </S>
  ),
  Help: (p: P) => (
    <S {...p}>
      <circle cx="8" cy="8" r="6.5" fill="url(#ig-blue)" stroke="#183f7a" strokeWidth="0.8" />
      <path d="M6 6.2a2 2 0 1 1 2.8 1.8c-.6.3-.8.7-.8 1.4v.4" stroke="#fff" strokeWidth="1.5" fill="none" />
      <circle cx="8" cy="11.8" r="0.9" fill="#fff" />
    </S>
  ),
  Close: (p: P) => (
    <svg width={p.size ?? 8} height={p.size ?? 8} viewBox="0 0 8 8" className={p.className} aria-hidden>
      <path d="M1 1l6 6M7 1L1 7" stroke="#333" strokeWidth="1.3" />
    </svg>
  ),
  Find: (p: P) => (
    <S {...p}>
      <circle cx="6.5" cy="6.5" r="4.3" fill="#e8f4ff" stroke="#1c4f9c" strokeWidth="1.5" />
      <path d="M9.6 9.6l4.6 4.6" stroke="#5c6878" strokeWidth="2.4" strokeLinecap="round" />
    </S>
  ),
  Follow: (p: P) => (
    <S {...p}>
      <path d="M2 8h10" stroke="#1c4f9c" strokeWidth="1.6" />
      <path d="M10 4.5l4 3.5-4 3.5z" fill="url(#ig-blue)" />
      <path d="M14.5 2v12" stroke="#333" />
    </S>
  ),
  ZoomIn: (p: P) => (
    <S {...p}>
      <circle cx="6.5" cy="6.5" r="4.6" fill="#fff" stroke="#1c4f9c" strokeWidth="1.4" />
      <path d="M4.2 6.5h4.6M6.5 4.2v4.6" stroke="#1c4f9c" strokeWidth="1.3" />
      <path d="M10 10l4.2 4.2" stroke="#5c6878" strokeWidth="2.2" strokeLinecap="round" />
    </S>
  ),
  ZoomOut: (p: P) => (
    <S {...p}>
      <circle cx="6.5" cy="6.5" r="4.6" fill="#fff" stroke="#1c4f9c" strokeWidth="1.4" />
      <path d="M4.2 6.5h4.6" stroke="#1c4f9c" strokeWidth="1.3" />
      <path d="M10 10l4.2 4.2" stroke="#5c6878" strokeWidth="2.2" strokeLinecap="round" />
    </S>
  ),
  ZoomFit: (p: P) => (
    <S {...p}>
      <rect x="1.5" y="3.5" width="13" height="9" fill="#fff" stroke="#1c4f9c" />
      <path d="M3 8h10M3 8l2-2M3 8l2 2M13 8l-2-2M13 8l-2 2" stroke="#1c4f9c" strokeWidth="1" fill="none" />
    </S>
  ),
  Fuse: (p: P) => (
    <S {...p}>
      <rect x="1.5" y="5.5" width="13" height="5" rx="2.5" fill="url(#ig-steel)" stroke="#5c6878" />
      <rect x="4.5" y="5.5" width="7" height="5" fill="#e8f2fb" stroke="#5c6878" strokeWidth="0.6" />
      <path d="M4.5 8c1.5-2 2.5 2 3.5 0s2 2 3.5 0" stroke="#c4561c" strokeWidth="0.9" fill="none" />
    </S>
  ),
  Chip3D: (p: P) => (
    <S {...p}>
      <path d="M8 1.5l6.5 3.6v5.8L8 14.5 1.5 10.9V5.1z" fill="url(#ig-chip)" stroke="#11161c" strokeWidth="0.6" />
      <path d="M8 3.6l4.3 2.4L8 8.4 3.7 6z" fill="#8fb9e8" stroke="#2a5ea8" strokeWidth="0.5" />
      <path d="M5.2 6l2.8 1.6L10.8 6" stroke="#ffd23b" strokeWidth="0.7" fill="none" />
      <path d="M8 8.4v5.9" stroke="#5c6878" strokeWidth="0.6" />
    </S>
  ),
  Book: (p: P) => (
    <S {...p}>
      <path d="M2.5 2.5h8.5a2 2 0 0 1 2 2v9.5H4.5a2 2 0 0 1-2-2z" fill="url(#ig-blue)" stroke="#183f7a" strokeWidth="0.8" />
      <path d="M4.5 11.5h8.5v2.5H4.5a1.25 1.25 0 0 1 0-2.5z" fill="#fff" stroke="#183f7a" strokeWidth="0.6" />
      <path d="M5 5h5.5M5 7h4" stroke="#fff" strokeWidth="0.9" />
    </S>
  ),
  Float: (p: P) => (
    <S {...p}>
      <rect x="1.5" y="2.5" width="10" height="8" fill="#fff" stroke="#5c6878" />
      <rect x="4.5" y="5.5" width="10" height="8" fill="url(#ig-paper)" stroke="#1c4f9c" />
      <rect x="4.5" y="5.5" width="10" height="2" fill="url(#ig-blue)" />
    </S>
  ),
  PopOut: (p: P) => (
    <S {...p}>
      <rect x="1.5" y="4.5" width="10" height="10" fill="#fff" stroke="#5c6878" />
      <path d="M8 1.5h6.5V8" fill="none" stroke="#1c4f9c" strokeWidth="1.5" />
      <path d="M14 2L7 9" stroke="#1c4f9c" strokeWidth="1.5" />
    </S>
  ),
  Dock: (p: P) => (
    <S {...p}>
      <path d="M6 1.5h4l-.5 5 2 2v1H4.5v-1l2-2z" fill="url(#ig-steel)" stroke="#5c6878" strokeWidth="0.8" />
      <path d="M8 9.5v5" stroke="#5c6878" strokeWidth="1.3" />
    </S>
  ),
  Generator: (p: P) => (
    <S {...p}>
      <rect x="1" y="2.5" width="14" height="11" rx="1.5" fill="#20303f" stroke="#11161c" strokeWidth="0.6" />
      <path d="M2.5 10.5h2v-5h3v5h3v-5h3" stroke="#6fe36b" strokeWidth="1.2" fill="none" />
    </S>
  ),
  Serial: (p: P) => (
    <S {...p}>
      <rect x="1.5" y="2.5" width="13" height="10" rx="1" fill="#20303f" stroke="#11161c" strokeWidth="0.6" />
      <path d="M3.5 5.5l2 1.5-2 1.5" stroke="#6fe36b" strokeWidth="1.1" fill="none" />
      <path d="M7 9h4" stroke="#6fe36b" strokeWidth="1.1" />
      <rect x="5" y="12.5" width="6" height="2" fill="url(#ig-steel)" stroke="#5c6878" strokeWidth="0.5" />
    </S>
  ),
  Download: (p: P) => (
    <S {...p}>
      <path d="M6 1.5h4v5.5h3L8 12.5 3 7h3z" fill="url(#ig-green)" stroke="#1d7d15" strokeWidth="0.8" strokeLinejoin="round" />
      <path d="M2 12.5v2h12v-2" fill="none" stroke="#5c6878" strokeWidth="1.4" />
    </S>
  ),
  MachineCode: (p: P) => (
    <S {...p}>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="url(#ig-paper)" stroke="#7a8ea8" />
      <text x="4.3" y="9" fontSize="4.6" fontFamily="monospace" fill="#1c4f9c" fontWeight="bold">01</text>
      <text x="4.3" y="13.2" fontSize="4.6" fontFamily="monospace" fill="#9b1d0c" fontWeight="bold">10</text>
    </S>
  ),
};

export type IconName = keyof typeof Icons;

/** Caption button glyphs (white with dark halo, like Aero). */
export const CaptionGlyph = {
  Min: () => <svg width="10" height="10" viewBox="0 0 10 10"><rect x="1" y="6.5" width="8" height="2.2" fill="#fff" stroke="#203040" strokeWidth="0.6" /></svg>,
  Max: () => <svg width="10" height="10" viewBox="0 0 10 10"><rect x="1" y="1" width="8" height="7.5" fill="none" stroke="#fff" strokeWidth="1.8" /><rect x="0.3" y="0.3" width="9.4" height="8.9" fill="none" stroke="#203040" strokeWidth="0.5" /></svg>,
  Restore: () => <svg width="10" height="10" viewBox="0 0 10 10"><rect x="3" y="0.8" width="6.2" height="5.4" fill="none" stroke="#fff" strokeWidth="1.4" /><rect x="0.8" y="3.4" width="6.2" height="5.6" fill="#7aa1cc" stroke="#fff" strokeWidth="1.4" /></svg>,
  Close: () => <svg width="12" height="10" viewBox="0 0 12 10"><path d="M2 1l8 8M10 1L2 9" stroke="#203040" strokeWidth="3.2" strokeLinecap="round" /><path d="M2 1l8 8M10 1L2 9" stroke="#fff" strokeWidth="1.8" strokeLinecap="round" /></svg>,
};
