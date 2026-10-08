import { useEffect, useMemo, useRef, useState, type JSX } from 'react';
import { useSim } from '../state/sim';
import { useLayout } from '../state/layout';
import { openExternal } from '../backend/api';
import { buildFloorplan, hitTest } from '../chip/floorplan';
import { drawDieBase, drawDieLabels } from '../chip/dieArt';
import { formatHz, hex } from '../format';
import { Icons } from '../icons';
import { EmptyHint, Section } from './common';

const kb = (b: number) => (b >= 1024 ? `${b / 1024} KB` : `${b} bytes`);

/** Device overview: key specifications, memory map, peripherals, pin-out and die floorplan. */
export function DeviceInfoPanel(): JSX.Element {
  const spec = useSim((s) => s.spec);
  if (!spec) return <EmptyHint>No device loaded.</EmptyHint>;
  const io = spec.pins.filter((p) => p.kind === 'io');
  const grades = spec.speedGrades.map(([hz, v]) => `${formatHz(hz)} @ ${v.toFixed(1)}-${spec.vccRange[1].toFixed(1)} V`).join(', ');
  const rows: [string, string][] = [
    ['Family', spec.family],
    ['CPU core', `${spec.coreName} (8-bit AVR RISC, ${spec.features & 1 ? '16' : '32'} general purpose registers)`],
    ['Program memory', `${kb(spec.flashSize)} flash (${spec.flashSize / 2} instruction words)`],
    ['Data memory', `${spec.sramSize} bytes SRAM at ${hex(spec.sramStart, 4)}-${hex(spec.sramStart + spec.sramSize - 1, 4)}`],
    ['EEPROM', spec.eepromSize ? kb(spec.eepromSize) : 'none'],
    ['Package', `${spec.package}, ${spec.pins.length} pins (${io.length} I/O)`],
    ['Clock', `internal ${formatHz(spec.clock.internalHz)} RC, ${formatHz(spec.clock.slowHz)} low-power oscillator, external clock input; prescaler /1-/256 (default /${2 ** spec.clock.defaultPrescaleLog2})`],
    ['Speed grades', grades || '-'],
    ['Supply', `${spec.vccRange[0]}-${spec.vccRange[1]} V`],
    ['Interrupt vectors', `${spec.vectors.length}`],
    ['Signature', spec.signature.map((b) => hex(b)).join(' ')],
    ['Model source', spec.datasheet],
  ];
  return (
    <div className="panel">
      <div className="panel-scroll device-info">
        <div className="info-hero">
          <Icons.App size={32} />
          <div>
            <h2>{spec.name}</h2>
            <div className="dim">{spec.family} - {spec.package}</div>
          </div>
          <div className="grow" />
          <button className="w7-btn" onClick={() => useLayout.getState().show('chip')}><Icons.Chip3D size={14} /><span>Open 3D Chip View</span></button>
        </div>
        <Section title="Specifications">
          <table className="grid-table info-table">
            <tbody>
              {rows.map(([k, v]) => (
                <tr key={k}><td className="dim">{k}</td><td className="selectable">{v}</td></tr>
              ))}
            </tbody>
          </table>
        </Section>
        <Section title="Silicon die">
          <DieDiagram />
          {spec.die && (
            <p className="dim" style={{ margin: '6px 4px' }}>
              Die size {spec.die.widthUm} x {spec.die.heightUm} um. The block arrangement above is an illustrative floorplan generated from the device model, not a traced layout.{' '}
              <a className="link" onClick={() => void openExternal(spec.die!.photoUrl)}>See a real die photograph</a> ({spec.die.photoCredit}).
            </p>
          )}
        </Section>
        <Section title="Memory map (data space)">
          <MemoryMap />
        </Section>
        <Section title="Peripherals">
          <table className="grid-table info-table">
            <tbody>
              {spec.groups.map((g) => (
                <tr key={g.name}>
                  <td><b>{g.name}</b></td>
                  <td>{g.desc}</td>
                  <td className="dim mono">{spec.registers.filter((r) => r.group === g.name).map((r) => r.name).join(' ')}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Section>
        <Section title="Pins">
          <table className="grid-table info-table">
            <thead><tr><th>Pin</th><th>Name</th><th>Functions</th></tr></thead>
            <tbody>
              {spec.pins.map((p) => (
                <tr key={p.number}><td>{p.number}</td><td><b>{p.name}</b></td><td>{p.kind === 'io' ? p.functions.join(', ') : p.kind === 'vcc' ? 'Supply voltage' : 'Ground'}</td></tr>
              ))}
            </tbody>
          </table>
        </Section>
        <Section title="Interrupt vectors">
          <table className="grid-table info-table">
            <thead><tr><th>#</th><th>Address</th><th>Name</th><th>Source</th></tr></thead>
            <tbody>
              {spec.vectors.map((v) => (
                <tr key={v.index}><td>{v.index}</td><td className="mono">{hex(v.index * (spec.features & 16 ? 4 : 2), 4)}</td><td><b>{v.name}</b></td><td>{v.desc}</td></tr>
              ))}
            </tbody>
          </table>
        </Section>
      </div>
    </div>
  );
}

/** Static die drawing with block names; hover shows details. */
function DieDiagram(): JSX.Element {
  const spec = useSim((s) => s.spec)!;
  const plan = useMemo(() => buildFloorplan(spec), [spec]);
  const ref = useRef<HTMLCanvasElement>(null);
  const [tip, setTip] = useState<{ x: number; y: number; text: string } | null>(null);
  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    const draw = () => {
      const w = c.clientWidth;
      if (!w) return;
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      const s = w / plan.w;
      c.width = Math.round(w * dpr);
      c.height = Math.round(plan.h * s * dpr);
      c.style.height = `${plan.h * s}px`;
      const ctx = c.getContext('2d')!;
      ctx.setTransform(dpr * s, 0, 0, dpr * s, 0, 0);
      drawDieBase(ctx, plan, spec);
      drawDieLabels(ctx, plan);
    };
    draw();
    const ro = new ResizeObserver(draw);
    ro.observe(c);
    return () => ro.disconnect();
  }, [plan, spec]);
  return (
    <div className="die-diagram">
      <canvas
        ref={ref}
        onMouseMove={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          const s = plan.w / r.width;
          const h = hitTest(plan, (e.clientX - r.left) * s, (e.clientY - r.top) * s);
          const text = h?.pad ? `${h.pad.pin.name} bond pad (package pin ${h.pad.pin.number})` : h?.block ? `${h.block.label}: ${h.block.sub}` : '';
          setTip(text ? { x: e.clientX - r.left, y: e.clientY - r.top, text } : null);
        }}
        onMouseLeave={() => setTip(null)}
        onClick={() => useLayout.getState().show('chip')}
      />
      {tip && <div className="chip-tip" style={{ left: tip.x + 12, top: tip.y + 14 }}>{tip.text}</div>}
    </div>
  );
}

function MemoryMap(): JSX.Element {
  const spec = useSim((s) => s.spec)!;
  const regions: [string, number, number, string][] = [];
  if (spec.regsInDataSpace) regions.push(['Registers R0-R31', 0, 31, 'regs']);
  regions.push([`I/O registers (${spec.ioSize})`, spec.ioBase, spec.ioBase + spec.ioSize - 1, 'io']);
  if (spec.sramStart > spec.ioBase + spec.ioSize) regions.push(['Extended I/O', spec.ioBase + spec.ioSize, spec.sramStart - 1, 'io']);
  regions.push([`SRAM (${spec.sramSize} B)`, spec.sramStart, spec.sramStart + spec.sramSize - 1, 'sram']);
  if (spec.nvmMap) {
    regions.push(['NVM lock bits', spec.nvmMap.lock, spec.nvmMap.lock + 1, 'nvm']);
    regions.push(['Configuration (fuses)', spec.nvmMap.config, spec.nvmMap.config + 1, 'nvm']);
    regions.push(['Calibration', spec.nvmMap.calibration, spec.nvmMap.calibration + 1, 'nvm']);
    regions.push(['Device ID (signature)', spec.nvmMap.signature, spec.nvmMap.signature + 3, 'nvm']);
  }
  if (spec.flashMapBase !== null) regions.push([`Flash (mapped for LD, ${spec.flashSize} B)`, spec.flashMapBase, spec.flashMapBase + spec.flashSize - 1, 'flash']);
  regions.sort((a, b) => a[1] - b[1]);
  return (
    <div className="mem-map">
      {regions.map(([name, a, b, kind]) => (
        <div key={name} className={`mem-region ${kind}`}>
          <span className="mono">{hex(a, 4)}-{hex(b, 4)}</span>
          <span>{name}</span>
        </div>
      ))}
    </div>
  );
}
