import { useEffect, useMemo, useRef, useState, type JSX } from 'react';
import { useSim } from '../state/sim';
import { clearSerial, sendSerial, serialConfigFor, setSerialConfig, setSerialPrefs, useSerial } from '../state/serial';
import { gpioNames } from '../services/device';
import { Icons } from '../icons';
import { EmptyHint } from './common';
import type { SerialConfig } from '../backend/types';

const BAUDS = [300, 1200, 2400, 4800, 9600, 14400, 19200, 38400, 57600, 115200, 230400, 250000, 500000, 1000000];
const FORMATS: [string, Pick<SerialConfig, 'dataBits' | 'parity' | 'stopBits'>][] = [
  ['8N1', { dataBits: 8, parity: 0, stopBits: 1 }],
  ['8N2', { dataBits: 8, parity: 0, stopBits: 2 }],
  ['8E1', { dataBits: 8, parity: 1, stopBits: 1 }],
  ['8O1', { dataBits: 8, parity: 2, stopBits: 1 }],
  ['7E1', { dataBits: 7, parity: 1, stopBits: 1 }],
  ['9N1', { dataBits: 9, parity: 0, stopBits: 1 }],
];

/** Text view of received bytes: printable ASCII, CR/LF line breaks, other bytes as <xx>. */
function render(bytes: Uint8Array, n: number, hex: boolean): string {
  if (hex) {
    let out = '';
    for (let i = 0; i < n; i++) out += bytes[i].toString(16).toUpperCase().padStart(2, '0') + ((i & 15) === 15 ? '\n' : ' ');
    return out;
  }
  let out = '';
  for (let i = 0; i < n; i++) {
    const b = bytes[i];
    if (b === 13) {
      if (bytes[i + 1] !== 10) out += '\n';
    } else if (b === 10 || b === 9 || (b >= 32 && b < 127)) out += String.fromCharCode(b);
    else out += `<${b.toString(16).toUpperCase().padStart(2, '0')}>`;
  }
  return out;
}

/** Serial terminal: decodes frames from a pin (the MCU's TX) and sends typed text into another pin (RX). */
export function SerialPanel(): JSX.Element {
  const spec = useSim((s) => s.spec);
  const version = useSerial((s) => s.version);
  const hex = useSerial((s) => s.hex);
  const lineEnding = useSerial((s) => s.lineEnding);
  const autoscroll = useSerial((s) => s.autoscroll);
  useSerial((s) => s.configs);
  const [input, setInput] = useState('');
  const [history, setHistory] = useState<string[]>([]);
  const [histPos, setHistPos] = useState(-1);
  const out = useRef<HTMLPreElement>(null);
  const text = useMemo(() => {
    const s = useSerial.getState();
    return render(s.bytes, s.length, hex);
  }, [version, hex]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (autoscroll && out.current) out.current.scrollTop = out.current.scrollHeight;
  }, [text, autoscroll]);

  if (!spec) return <EmptyHint>No device loaded.</EmptyHint>;
  const cfg = serialConfigFor(spec);
  const names = gpioNames(spec);
  const set = (patch: Partial<SerialConfig>) => setSerialConfig(spec, { ...cfg, ...patch });
  const fmt = FORMATS.find(([, f]) => f.dataBits === cfg.dataBits && f.parity === cfg.parity && f.stopBits === cfg.stopBits)?.[0] ?? 'custom';
  const pinLabel = (g: number) => {
    const p = spec.pins.find((x) => x.gpio === g);
    const fn = p?.functions.find((f) => f === 'TXD' || f === 'RXD');
    return `${names[g]}${fn ? ` (${fn})` : ''}`;
  };
  const send = () => {
    sendSerial(input);
    if (input) setHistory((h) => [input, ...h.filter((x) => x !== input)].slice(0, 30));
    setHistPos(-1);
    setInput('');
  };
  return (
    <div className="panel serial-panel">
      <div className="panel-toolbar">
        <span data-tip="Pin decoded into this window (the microcontroller's TX)">From</span>
        <select className="w7-select" value={cfg.monitor ?? ''} onChange={(e) => set({ monitor: e.target.value === '' ? null : Number(e.target.value) })}>
          <option value="">(none)</option>
          {names.map((_, g) => <option key={g} value={g}>{pinLabel(g)}</option>)}
        </select>
        <span data-tip="Pin that receives what you type (the microcontroller's RX)">To</span>
        <select className="w7-select" value={cfg.inject ?? ''} onChange={(e) => set({ inject: e.target.value === '' ? null : Number(e.target.value) })}>
          <option value="">(none)</option>
          {names.map((_, g) => <option key={g} value={g}>{pinLabel(g)}</option>)}
        </select>
        <select className="w7-select" value={cfg.baud} onChange={(e) => set({ baud: Number(e.target.value) })} data-tip="Baud rate">
          {(BAUDS.includes(cfg.baud) ? BAUDS : [...BAUDS, cfg.baud]).map((b) => <option key={b} value={b}>{b} baud</option>)}
        </select>
        <select className="w7-select" value={fmt} onChange={(e) => { const f = FORMATS.find(([n]) => n === e.target.value); if (f) set(f[1]); }} data-tip="Data bits, parity, stop bits">
          {FORMATS.map(([n]) => <option key={n} value={n}>{n}</option>)}
          {fmt === 'custom' && <option value="custom">custom</option>}
        </select>
        <div className="grow" />
        <label className="w7-check" data-tip="Show bytes in hexadecimal">
          <input type="checkbox" checked={hex} onChange={(e) => setSerialPrefs({ hex: e.target.checked })} /> Hex
        </label>
        <label className="w7-check" data-tip="Scroll to new output">
          <input type="checkbox" checked={autoscroll} onChange={(e) => setSerialPrefs({ autoscroll: e.target.checked })} /> Autoscroll
        </label>
        <button className="tb-btn" data-tip="Clear" onClick={clearSerial}><Icons.Clear /></button>
      </div>
      <pre className="serial-out selectable mono" ref={out}>
        {text || <span className="dim">{cfg.monitor === null ? 'Choose the pin to listen to (the MCU\'s TX) above.' : `Listening on ${names[cfg.monitor]} at ${cfg.baud} baud...`}</span>}
      </pre>
      <div className="serial-input">
        <input
          className="w7-input mono"
          placeholder={cfg.inject === null ? 'Choose the "To" pin to send text' : `Send to ${names[cfg.inject]} (Enter)`}
          disabled={cfg.inject === null}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') send();
            if (e.key === 'ArrowUp' && history.length) {
              const p = Math.min(histPos + 1, history.length - 1);
              setHistPos(p);
              setInput(history[p]);
              e.preventDefault();
            }
            if (e.key === 'ArrowDown') {
              const p = histPos - 1;
              setHistPos(Math.max(-1, p));
              setInput(p >= 0 ? history[p] : '');
              e.preventDefault();
            }
          }}
        />
        <select className="w7-select" value={JSON.stringify(lineEnding)} onChange={(e) => setSerialPrefs({ lineEnding: JSON.parse(e.target.value) })} data-tip="Appended to every line you send">
          <option value={JSON.stringify('')}>No line ending</option>
          <option value={JSON.stringify('\n')}>Newline (LF)</option>
          <option value={JSON.stringify('\r')}>Carriage return (CR)</option>
          <option value={JSON.stringify('\r\n')}>CR + LF</option>
        </select>
        <button className="w7-btn small" disabled={cfg.inject === null} onClick={send}><span>Send</span></button>
      </div>
    </div>
  );
}
