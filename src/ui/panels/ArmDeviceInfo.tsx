import type { JSX } from 'react';
import { armHasDouble, armHasFpu, type ArmDeviceSpec } from '../backend/types';
import { formatHz, hex } from '../format';
import { Icons } from '../icons';
import { Section } from './common';

const kb = (b: number) => (b >= 1024 ? `${b / 1024} KB` : `${b} bytes`);

/** Device overview for ARM Cortex-M parts (no die floorplan: the Chip View is AVR-only). */
export function ArmDeviceInfo({ spec }: { spec: ArmDeviceSpec }): JSX.Element {
  const io = spec.pins.filter((p) => p.kind === 'io');
  const fpu = armHasFpu(spec) ? (armHasDouble(spec) ? 'FPv5 double + single precision' : 'FPv4 single precision') : 'no FPU';
  const dsp = spec.features & 1 ? ', DSP extension' : '';
  const grades = spec.speedGrades.map(([hz, v]) => `${formatHz(hz)} @ ${v.toFixed(2)}-${spec.vccRange[1].toFixed(1)} V`).join(', ');
  const ccm = spec.ccmSram;
  const rows: [string, string][] = [
    ['Family', spec.family],
    ['CPU core', `${spec.coreName} (32-bit ARMv7-M, Thumb-2, ${fpu}${dsp}, CPUID ${hex(spec.cpuid, 8)})`],
    ['Flash', `${kb(spec.flashSize)} at ${hex(spec.flashBase, 8)}`],
    ['SRAM', `${kb(spec.sramSize)} at ${hex(spec.sramBase, 8)}${ccm ? `, CCM SRAM ${kb(ccm.size)} at ${hex(ccm.base, 8)} (also at ${hex(ccm.aliasBase, 8)})` : ''}`],
    ['Package', `${spec.package}, ${spec.pins.length} pins (${io.length} I/O)`],
    ['Clock', `HSI ${formatHz(spec.clock.hsiHz)}, LSI ${formatHz(spec.clock.lsiHz)}, HSE ${formatHz(spec.clock.hseMinHz)}-${formatHz(spec.clock.hseMaxHz)} (default ${formatHz(spec.clock.hseDefaultHz)}), PLL`],
    ['Speed grades', grades || '-'],
    ['Supply', `${spec.vccRange[0]}-${spec.vccRange[1]} V`],
    ['Interrupts', `${spec.nirq} external lines, ${spec.nvicPrioBits} priority bits`],
    ['Model source', spec.datasheet],
  ];
  const regions: [string, number, number][] = [
    ['Flash', spec.flashBase, spec.flashBase + spec.flashSize - 1],
    ['SRAM', spec.sramBase, spec.sramBase + spec.sramSize - 1],
    ...(ccm ? ([['CCM SRAM', ccm.base, ccm.base + ccm.size - 1]] as [string, number, number][]) : []),
    ['Peripherals', 0x40000000, 0x5fffffff],
    ['Core peripherals (SCS, NVIC, SysTick)', 0xe0000000, 0xe00fffff],
  ];
  regions.sort((a, b) => a[1] - b[1]);
  return (
    <div className="panel">
      <div className="panel-scroll device-info">
        <div className="info-hero">
          <Icons.App size={32} />
          <div>
            <h2>{spec.name}</h2>
            <div className="dim">{spec.family} - {spec.package}</div>
          </div>
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
        <Section title="Memory map">
          <div className="mem-map">
            {regions.map(([name, a, b]) => (
              <div key={name} className="mem-region sram">
                <span className="mono">{hex(a, 8)}-{hex(b, 8)}</span>
                <span>{name}</span>
              </div>
            ))}
          </div>
        </Section>
        <Section title="Peripherals">
          <table className="grid-table info-table">
            <tbody>
              {spec.groups.map((g) => (
                <tr key={g.name}>
                  <td><b>{g.name}</b></td>
                  <td>{g.desc}</td>
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
                <tr key={p.number}><td>{p.number}</td><td><b>{p.name}</b></td><td>{p.kind === 'io' ? p.functions.join(', ') : p.kind === 'vcc' ? 'Supply voltage' : p.kind === 'gnd' ? 'Ground' : 'Reference'}</td></tr>
              ))}
            </tbody>
          </table>
        </Section>
        <Section title="Exceptions and interrupts">
          <table className="grid-table info-table">
            <thead><tr><th>#</th><th>Vector address</th><th>Name</th><th>Source</th></tr></thead>
            <tbody>
              {spec.vectors.map((v) => (
                <tr key={v.index}><td>{v.index}</td><td className="mono">{hex(v.index * 4, 4)}</td><td><b>{v.name}</b></td><td>{v.desc}</td></tr>
              ))}
            </tbody>
          </table>
        </Section>
      </div>
    </div>
  );
}
