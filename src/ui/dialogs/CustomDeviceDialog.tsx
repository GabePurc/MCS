/**
 * Custom microcontroller editor (issue #3): pick memory sizes, ports and peripheral counts; the
 * Rust backend validates the configuration and generates the device (registers, vectors, pins,
 * wiring). Only architectural limits apply (they are reported by the backend).
 */
import { useEffect, useState, type JSX } from 'react';
import { Dialog } from './Dialogs';
import { closeDialog } from '../state/dialogs';
import { customDeviceDefaults, customDevicePreview } from '../backend/api';
import type { CustomMcuConfig, CustomPreview } from '../backend/types';
import { customId, deleteCustomDevice, saveCustomDevice, useCustomDevices } from '../state/customDevices';
import { loadDevices, useDevices } from '../state/devices';
import { useSettings } from '../state/settings';
import { selectDevice } from '../services/device';
import { formatHz, hex, parseHz } from '../format';

type NumKey = { [K in keyof CustomMcuConfig]: CustomMcuConfig[K] extends number ? K : never }[keyof CustomMcuConfig];

/** "32K", "2 KB", "8M", "0x800", "3000" -> bytes (K/M are binary multiples). */
function parseSize(text: string): number {
  const m = /^\s*(0x[0-9a-f]+|\d+(?:\.\d+)?)\s*([kmg]?)i?b?\s*$/i.exec(text);
  if (!m) return NaN;
  const v = m[1].toLowerCase().startsWith('0x') ? parseInt(m[1], 16) : Number(m[1]);
  return Math.round(v * { '': 1, k: 1024, m: 1024 ** 2, g: 1024 ** 3 }[m[2].toLowerCase() as '' | 'k' | 'm' | 'g']);
}

function formatSize(n: number): string {
  if (n >= 1024 ** 2 && n % 1024 ** 2 === 0) return `${n / 1024 ** 2}M`;
  if (n >= 1024 && n % 1024 === 0) return `${n / 1024}K`;
  return String(n);
}

/** Non-GPIO package pins (mirrors mcs_core custom::DEDICATED_PINS). */
const DEDICATED = ['VCC', 'GND', 'AVCC', 'AREF', 'RESET', 'XTAL1', 'XTAL2', 'NC'];
const PORT_LETTERS = 'ABCDEFGHJKLMNOPQRSTUVWXYZ';

/** GPIO `g` name as the backend generates it ("PB3"; ports A..Z without I, then AA, AB...). */
function gpioName(g: number): string {
  const p = Math.floor(g / 8);
  return `P${p < PORT_LETTERS.length ? PORT_LETTERS[p] : `A${PORT_LETTERS[p - PORT_LETTERS.length]}`}${g % 8}`;
}

const gpioCount = (c: CustomMcuConfig) => c.gpioPins || c.ports * 8;

/** Keeps an explicit pinout valid after the GPIO count or ADC changed: drops GPIOs that no longer exist, appends new ones (and AREF when an ADC needs it), pads to an even count with NC. */
export function reconcilePinout(c: CustomMcuConfig): string[] {
  if (!c.pinout.length) return c.pinout;
  const names = Array.from({ length: gpioCount(c) }, (_, g) => gpioName(g));
  const out = c.pinout.filter((n) => DEDICATED.includes(n) || names.includes(n));
  for (const n of names) if (!out.includes(n)) out.push(n);
  if (c.adcChannels > 0 && !out.includes('AREF')) out.push('AREF');
  const nc = out.lastIndexOf('NC');
  if (out.length % 2) {
    if (nc >= 0 && nc === out.length - 1) out.pop();
    else out.push('NC');
  }
  return out;
}

const PRESETS: [string, Partial<CustomMcuConfig>][] = [
  ['Tiny', { flashSize: 1024, sramSize: 64, eepromSize: 0, ports: 1, gpioPins: 8, extInterrupts: 1, timers8: 1, timers16: 0, usarts: 0, spis: 0, twis: 0, adcChannels: 0, analogComparator: false, hardwareMultiplier: false }],
  ['ATmega328P-like', { flashSize: 32768, sramSize: 2048, eepromSize: 1024, ports: 3, gpioPins: 24, extInterrupts: 2, timers8: 2, timers16: 1, usarts: 1, spis: 1, twis: 1, adcChannels: 6, analogComparator: true, hardwareMultiplier: true }],
  ['ATmega2560-like', { flashSize: 262144, sramSize: 8192, eepromSize: 4096, ports: 11, gpioPins: 88, extInterrupts: 8, timers8: 2, timers16: 4, usarts: 4, spis: 1, twis: 1, adcChannels: 16, analogComparator: true, hardwareMultiplier: true, package: 'SOIC' }],
  ['Huge', { flashSize: 8 * 1024 * 1024, sramSize: 60 * 1024, eepromSize: 61440, ports: 26, gpioPins: 208, extInterrupts: 16, timers8: 8, timers16: 8, usarts: 8, spis: 4, twis: 4, adcChannels: 30, analogComparator: true, hardwareMultiplier: true, package: 'SOIC' }],
];

function SizeField({ value, onChange, tip }: { value: number; onChange: (n: number) => void; tip: string }): JSX.Element {
  const [text, setText] = useState(formatSize(value));
  useEffect(() => setText((t) => (parseSize(t) === value ? t : formatSize(value))), [value]);
  return (
    <span className="cmcu-size">
      <input className={`w7-input mono${Number.isNaN(parseSize(text)) ? ' invalid' : ''}`} value={text} data-tip={tip} onChange={(e) => { setText(e.target.value); const n = parseSize(e.target.value); if (!Number.isNaN(n)) onChange(n); }} />
      <span className="dim">{value.toLocaleString()} B</span>
    </span>
  );
}

export function CustomDeviceDialog(): JSX.Element {
  const configs = useCustomDevices((s) => s.configs);
  const current = useSettings((s) => s.deviceId);
  const [editing, setEditing] = useState<string>(() => (configs.some((c) => c.id === current) ? current : ''));
  const [cfg, setCfg] = useState<CustomMcuConfig | null>(null);
  const [preview, setPreview] = useState<{ ok?: CustomPreview; error?: string } | null>(null);
  const [hzText, setHzText] = useState({ internal: '', max: '' });
  const [saveError, setSaveError] = useState<string | null>(null);
  const [advanced, setAdvanced] = useState(false);
  // Last valid preview: the advanced pin editor keeps working while an edit is invalid.
  const [lastOk, setLastOk] = useState<CustomPreview | null>(null);

  // Load the selected configuration (or defaults for a new one).
  useEffect(() => {
    const existing = configs.find((c) => c.id === editing);
    if (existing) {
      setCfg({ ...existing, gpioPins: existing.gpioPins ?? 0, pinout: existing.pinout ?? [], pinFunctions: existing.pinFunctions ?? {} });
      setAdvanced(!!existing.pinout?.length || Object.keys(existing.pinFunctions ?? {}).length > 0);
      setHzText({ internal: formatHz(existing.internalHz), max: formatHz(existing.maxHz) });
      return;
    }
    void customDeviceDefaults().then((d) => {
      setCfg({ ...d, id: '', name: 'My MCU' });
      setHzText({ internal: formatHz(d.internalHz), max: formatHz(d.maxHz) });
    });
  }, [editing]); // eslint-disable-line react-hooks/exhaustive-deps

  // Live validation / summary from the backend (debounced).
  useEffect(() => {
    if (!cfg) return;
    const t = setTimeout(() => void customDevicePreview({ ...cfg, id: cfg.id || 'custom-preview' }).then((r) => { setPreview(r); if (r.ok) setLastOk(r.ok); }), 120);
    return () => clearTimeout(t);
  }, [cfg]);

  if (!cfg) return <Dialog title="Custom Microcontroller" width={720}><p>Loading...</p></Dialog>;
  const set = <K extends keyof CustomMcuConfig>(k: K, v: CustomMcuConfig[K]) => {
    setSaveError(null);
    const next = { ...cfg, [k]: v };
    setCfg(k === 'adcChannels' ? { ...next, pinout: reconcilePinout(next) } : next);
  };
  const num = (k: NumKey, label: string, tip: string, max?: number) => (
    <>
      <span>{label}:</span>
      <input type="number" className="w7-input mono cmcu-num" min={0} max={max} value={cfg[k]} data-tip={tip} onChange={(e) => set(k, Math.max(0, Math.floor(Number(e.target.value) || 0)))} />
    </>
  );
  const save = async (select: boolean) => {
    const id = cfg.id || customId(cfg.name, (x) => configs.some((c) => c.id === x) || useDevices.getState().devices.some((d) => d.id === x));
    const next = { ...cfg, id };
    const err = await saveCustomDevice(next);
    if (err) {
      setSaveError(err);
      return;
    }
    await loadDevices();
    setCfg(next);
    setEditing(id);
    if (select || current === id) {
      // Re-selecting the same id must rebuild the machine from the new spec.
      if (current === id) useSettings.getState().set({ deviceId: '' });
      selectDevice(id);
      closeDialog();
    }
  };
  const remove = () => {
    if (!cfg.id) return;
    deleteCustomDevice(cfg.id);
    if (current === cfg.id) selectDevice('atmega328p');
    void loadDevices();
    setEditing('');
  };
  const p = preview?.ok;
  const error = saveError ?? preview?.error ?? null;
  return (
    <Dialog
      title="Custom Microcontroller"
      width={760}
      gray
      buttons={
        <>
          {cfg.id && <button className="w7-btn left" onClick={remove}><span>Delete</span></button>}
          <button className="w7-btn default" disabled={!!preview?.error} onClick={() => void save(true)}><span>Save and Use</span></button>
          <button className="w7-btn" disabled={!!preview?.error} onClick={() => void save(false)}><span>Save</span></button>
          <button className="w7-btn" onClick={closeDialog}><span>Close</span></button>
        </>
      }
    >
      <div className="cmcu-top">
        <span>Device:</span>
        <select className="w7-select" value={editing} onChange={(e) => setEditing(e.target.value)}>
          <option value="">(new custom device)</option>
          {configs.map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}
        </select>
        <span>Name:</span>
        <input className="w7-input grow" value={cfg.name} onChange={(e) => set('name', e.target.value)} />
      </div>
      <div className="cmcu-top">
        <span>Start from:</span>
        {PRESETS.map(([label, patch]) => (
          <button key={label} className="seg-btn" onClick={() => { setSaveError(null); const next = { ...cfg, ...patch }; setCfg({ ...next, pinout: reconcilePinout(next) }); }}>{label}</button>
        ))}
      </div>
      <div className="cmcu-cols">
        <div className="w7-group">
          <div className="w7-group-title">Memory</div>
          <div className="form-grid">
            <span>Flash:</span><SizeField value={cfg.flashSize} onChange={(n) => set('flashSize', n)} tip="Program memory in bytes (any even size; K/M suffixes allowed). Up to 8 MB: the AVR program counter is 22 bits." />
            <span>SRAM:</span><SizeField value={cfg.sramSize} onChange={(n) => set('sramSize', n)} tip="Data memory in bytes. Registers + I/O + SRAM share the 64 KB data space (16-bit addresses)." />
            <span>EEPROM:</span><SizeField value={cfg.eepromSize} onChange={(n) => set('eepromSize', n)} tip="0 = no EEPROM. Up to 65535 B (16-bit EEAR)." />
          </div>
        </div>
        <div className="w7-group">
          <div className="w7-group-title">Pins</div>
          <div className="form-grid">
            <span>GPIO pins:</span>
            <input
              type="number" className="w7-input mono cmcu-num" min={1} max={248} value={gpioCount(cfg)}
              data-tip="Number of I/O pins (1-248). They are grouped into 8-pin ports A, B, C... (I is skipped); the last port may be partial."
              onChange={(e) => {
                const n = Math.max(1, Math.min(248, Math.floor(Number(e.target.value) || 1)));
                const next = { ...cfg, gpioPins: n, ports: Math.ceil(n / 8) };
                setSaveError(null);
                setCfg({ ...next, pinout: reconcilePinout(next) });
              }}
            />
            <span>Package:</span>
            <select className="w7-select" value={cfg.package} onChange={(e) => set('package', e.target.value as CustomMcuConfig['package'])}>
              <option value="DIP">DIP (through-hole)</option>
              <option value="SOIC">SOIC (surface mount)</option>
            </select>
            {num('extInterrupts', 'INT pins', 'External interrupt pins (INT0, INT1...). Every port also gets a pin-change interrupt group.')}
          </div>
        </div>
        <div className="w7-group">
          <div className="w7-group-title">Peripherals</div>
          <div className="form-grid cmcu-two">
            {num('timers8', '8-bit timers', '8-bit Timer/Counters with two compare outputs (like TC0/TC2)')}
            {num('timers16', '16-bit timers', '16-bit Timer/Counters with input capture (like TC1)')}
            {num('usarts', 'USARTs', 'Serial ports (USART0, USART1...)')}
            {num('spis', 'SPI', 'SPI controllers')}
            {num('twis', 'TWI (I2C)', 'Two-wire interfaces')}
            {num('adcChannels', 'ADC channels', '0 = no ADC; up to 30 input pins (MUX4:0 has 32 codes; 30 = bandgap, 31 = GND)', 30)}
          </div>
          <label className="w7-check"><input type="checkbox" checked={cfg.analogComparator} onChange={(e) => set('analogComparator', e.target.checked)} /> Analog comparator</label>
        </div>
        <div className="w7-group">
          <div className="w7-group-title">Core and clock</div>
          <label className="w7-check"><input type="checkbox" checked={cfg.hardwareMultiplier} onChange={(e) => set('hardwareMultiplier', e.target.checked)} /> Hardware multiplier (MUL, MULS, FMUL...)</label>
          <div className="form-grid">
            <span>Internal RC:</span>
            <input className="w7-input mono" value={hzText.internal} onChange={(e) => { setHzText({ ...hzText, internal: e.target.value }); const v = parseHz(e.target.value); if (v > 0) set('internalHz', v); }} />
            <span>Max clock:</span>
            <input className="w7-input mono" value={hzText.max} data-tip="Speed grade: the Supply & Clock dialog warns above it" onChange={(e) => { setHzText({ ...hzText, max: e.target.value }); const v = parseHz(e.target.value); if (v > 0) set('maxHz', v); }} />
            <span>VCC:</span>
            <input type="number" step={0.1} min={1} max={6} className="w7-input mono cmcu-num" value={cfg.vcc} onChange={(e) => set('vcc', Number(e.target.value) || 5)} />
          </div>
        </div>
      </div>
      <label className="w7-check cmcu-adv-toggle"><input type="checkbox" checked={advanced} onChange={(e) => setAdvanced(e.target.checked)} /> Advanced: pin layout and pin functions</label>
      {advanced && lastOk && <AdvancedPins cfg={cfg} preview={lastOk} onChange={(patch) => { setSaveError(null); setCfg({ ...cfg, ...patch }); }} />}
            <div className={`cmcu-summary${error ? ' error' : ''}`}>
        {error ? (
          <span className="error-text">{error}</span>
        ) : p ? (
          <>
            <b>{p.coreName}</b>, {p.package} ({p.pins} pins, {p.gpios} GPIO) - {p.registers} I/O registers, {p.vectors} interrupt vectors - SRAM {hex(p.sramStart, 4)}..{hex(p.ramEnd, 4)}
            <div className="dim">{p.groups.join(', ')}</div>
          </>
        ) : (
          '...'
        )}
      </div>
    </Dialog>
  );
}

/** Advanced pin editor: explicit package pinout (what every pin is) and per-function GPIO placement. */
function AdvancedPins({ cfg, preview, onChange }: { cfg: CustomMcuConfig; preview: CustomPreview; onChange: (patch: Partial<CustomMcuConfig>) => void }): JSX.Element {
  const gpios = Array.from({ length: gpioCount(cfg) }, (_, g) => gpioName(g));
  const custom = cfg.pinout.length > 0;
  const order = custom ? cfg.pinout : preview.pinList.map((p) => p.name);
  const fnsOf = (name: string) => preview.pinList.find((p) => p.name === name)?.functions.filter((f) => f !== name).join(', ') ?? '';
  const setPin = (i: number, name: string) => {
    const next = [...order];
    const j = gpios.includes(name) ? next.indexOf(name) : -1;
    if (j >= 0) next[j] = next[i]; // picking a GPIO used elsewhere swaps the two pins
    next[i] = name;
    onChange({ pinout: next });
  };
  const move = (i: number, d: number) => {
    const next = [...order];
    [next[i], next[i + d]] = [next[i + d], next[i]];
    onChange({ pinout: next });
  };
  const assigned = Object.entries(cfg.pinFunctions);
  const stale = assigned.filter(([f, pin]) => !preview.assignable.includes(f) || !gpios.includes(pin)).map(([f]) => f);
  const setFn = (f: string, pin: string) => {
    const next = { ...cfg.pinFunctions };
    if (pin) next[f] = pin;
    else delete next[f];
    onChange({ pinFunctions: next });
  };
  const autoPin = (f: string) => preview.pinList.find((p) => p.gpio && p.functions.includes(f))?.name ?? '?';
  return (
    <div className="cmcu-adv">
      <div className="w7-group">
        <div className="w7-group-title">Package pinout ({order.length} pins)</div>
        <label className="w7-check">
          <input type="checkbox" checked={custom} onChange={(e) => onChange({ pinout: e.target.checked ? preview.pinList.map((p) => p.name) : [] })} /> Custom pin order (choose what every pin is)
        </label>
        <div className="cmcu-pin-list">
          {order.map((name, i) => (
            <div key={i} className="cmcu-pin-row">
              <span className="mono dim">{i + 1}</span>
              {custom ? (
                <select className="w7-select mono" value={name} onChange={(e) => setPin(i, e.target.value)}>
                  <optgroup label="Power and special">{DEDICATED.map((n) => <option key={n} value={n}>{n}</option>)}</optgroup>
                  <optgroup label="GPIO">{gpios.map((n) => <option key={n} value={n}>{n}</option>)}</optgroup>
                </select>
              ) : (
                <b className="mono">{name}</b>
              )}
              <span className="dim cmcu-pin-fns">{gpios.includes(name) ? fnsOf(name) : ''}</span>
              {custom && (
                <span className="cmcu-pin-btns">
                  <button className="seg-btn" disabled={i === 0} data-tip="Move up" onClick={() => move(i, -1)}>▲</button>
                  <button className="seg-btn" disabled={i === order.length - 1} data-tip="Move down" onClick={() => move(i, 1)}>▼</button>
                  <button className="seg-btn" disabled={gpios.includes(name)} data-tip="Remove pin (GPIOs cannot be removed; lower the GPIO count instead)" onClick={() => onChange({ pinout: order.filter((_, k) => k !== i) })}>✕</button>
                </span>
              )}
            </div>
          ))}
        </div>
        {custom && (
          <div className="cmcu-top">
            <button className="seg-btn" onClick={() => onChange({ pinout: [...order, 'NC'] })}>Add pin</button>
            <button className="seg-btn" onClick={() => onChange({ pinout: [...order, 'VCC', 'GND'] })}>Add VCC + GND</button>
            <span className="dim">Two-row packages need an even pin count.</span>
          </div>
        )}
      </div>
      <div className="w7-group">
        <div className="w7-group-title">Pin functions</div>
        <div className="dim">Choose the GPIO for a function (e.g. SDA, TXD0, ADC3); Auto places it like the generator.</div>
        <div className="cmcu-fn-grid">
          {[...preview.assignable, ...stale.filter((f) => !preview.assignable.includes(f))].map((f) => (
            <label key={f} className={stale.includes(f) ? 'error-text' : ''}>
              <span className="mono">{f}</span>
              <select className="w7-select mono" value={cfg.pinFunctions[f] ?? ''} onChange={(e) => setFn(f, e.target.value)}>
                <option value="">Auto ({autoPin(f)})</option>
                {gpios.map((n) => <option key={n} value={n}>{n}</option>)}
              </select>
            </label>
          ))}
        </div>
        {assigned.length > 0 && <button className="seg-btn" onClick={() => onChange({ pinFunctions: {} })}>All Auto</button>}
      </div>
    </div>
  );
}
