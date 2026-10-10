import type { JSX } from 'react';
import type { RiscvDeviceSpec } from '../backend/types';
import { formatHz, hex } from '../format';
import { Icons } from '../icons';
import { Section } from './common';

const kb = (b: number) => (b >= 1 << 20 ? `${b / (1 << 20)} MiB` : b >= 1024 ? `${b / 1024} KiB` : `${b} bytes`);

/** What the model leaves out or simplifies (see docs/MULTI_ARCH.md, Stage E2). */
const SIMPLIFICATIONS = [
  'Boot: the program starts at its entry point (direct boot); the ROM bootloader and the second stage bootloader are not run, and ROM functions are not simulated (calling one stops the simulation).',
  'Flash: the cache / MMU is a fixed linear mapping of the flash image into IROM and DROM; there are no wait states, no encryption or secure boot.',
  'Timing: the core is a single-issue approximation (loads 2, taken branches 3, division 33 cycles); caches are not modelled.',
  'Not modelled: SPI, I2C, I2S, LEDC, ADC, TWAI, RMT, DMA, crypto accelerators, Wi-Fi / Bluetooth, EFUSE, RNG, deep / light sleep and the RTC slow clock. Their registers read 0 and ignore writes (a warning is shown once per page).',
  'USB Serial/JTAG: only the device-to-host serial direction; JTAG and the USB host side are absent.',
  'Core: machine mode only (no user mode, PMP is stored but not enforced); misaligned loads and stores trap like on the real core.',
  'Watchdogs (TIMG, RTC, super watchdog) store their configuration but never fire.',
];

/** Device overview for the ESP32-C3 (RV32IMC). */
export function RiscvDeviceInfo({ spec }: { spec: RiscvDeviceSpec }): JSX.Element {
  const io = spec.pins.filter((p) => p.kind === 'io');
  const grades = spec.speedGrades.map(([hz, v]) => `${formatHz(hz)} @ ${v.toFixed(1)}-${spec.vccRange[1].toFixed(1)} V`).join(', ');
  const c = spec.clock;
  const rows: [string, string][] = [
    ['Family', spec.family],
    ['CPU core', `${spec.coreName}, ${spec.isa}, machine mode, ${spec.cpuInterrupts} interrupt lines (priorities 0-15)`],
    ['Flash', `${kb(spec.flashSize)} ${spec.flashExternal ? 'external SPI flash (assumed size)' : 'in-package SPI flash'}; code window IROM ${hex(spec.flashBase, 8)}, data window DROM ${hex(spec.dromBase, 8)}`],
    ['SRAM', `SRAM1 ${kb(spec.sramSize)}: DRAM ${hex(spec.sramBase, 8)} / IRAM ${hex(spec.iramBase, 8)} (the same bytes through the data and the instruction bus)`],
    ...(spec.extraRam.length ? ([['Other RAM', spec.extraRam.map((r) => `${r.name} ${kb(r.size)} at ${hex(r.base, 8)}`).join(', ')]] as [string, string][]) : []),
    ['Package', `${spec.package}, ${spec.pins.length - 1} pins + exposed pad; ${io.length} GPIO pads (GPIO0-GPIO21 without GPIO11, which shares the VDD_SPI pin${spec.flashExternal ? '' : '; GPIO12-GPIO17 connect to the in-package flash'})`],
    ['Clocks', `crystal ${formatHz(c.xtalHz)} (CPU runs on XTAL / 2 = ${formatHz(c.xtalHz / 2)} after reset), PLL 80 / 160 MHz (CPU up to ${formatHz(c.cpuMaxHz)}), RC_FAST ~${formatHz(c.rcFastHz)}, RC_SLOW ~${formatHz(c.rcSlowHz)}, SYSTIMER ${formatHz(c.systimerHz)}`],
    ['Speed grades', grades || '-'],
    ['Supply', `${spec.vccRange[0]}-${spec.vccRange[1]} V`],
    ['Strapping pins', spec.strapping.map((g) => `GPIO${g}`).join(', ')],
    ['Interrupts', `${spec.interrupts.length} sources routed by the interrupt matrix to ${spec.cpuInterrupts} CPU lines (mie / mip bits 1-31)`],
    ['Model source', spec.datasheet],
  ];
  const peripherals = spec.groups.map((g) => ({ ...g, regs: spec.registers.filter((r) => r.group === g.name).length }));
  const map = [...spec.memoryMap].sort((a, b) => a.base - b.base || a.name.localeCompare(b.name));
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
          <table className="grid-table info-table">
            <thead><tr><th>Address range</th><th>Region</th><th>Access</th><th>Notes</th></tr></thead>
            <tbody>
              {map.map((m) => (
                <tr key={`${m.name}-${m.base}`}>
                  <td className="mono" style={{ whiteSpace: 'nowrap' }}>{hex(m.base, 8)}-{hex(m.base + m.size - 1, 8)}</td>
                  <td><b>{m.name}</b> <span className="dim">{kb(m.size)}</span></td>
                  <td className="mono">{m.perm}</td>
                  <td>{m.desc}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Section>
        <Section title="Peripherals modelled">
          <table className="grid-table info-table">
            <tbody>
              {peripherals.map((g) => (
                <tr key={g.name}>
                  <td><b>{g.name}</b></td>
                  <td>{g.desc}</td>
                  <td className="dim">{g.regs} registers</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Section>
        <Section title="Simplifications">
          <ul className="info-list">
            {SIMPLIFICATIONS.map((t) => <li key={t}>{t}</li>)}
          </ul>
        </Section>
        <Section title="Pins">
          <table className="grid-table info-table">
            <thead><tr><th>Pin</th><th>Name</th><th>Functions</th></tr></thead>
            <tbody>
              {spec.pins.map((p) => (
                <tr key={p.number}><td>{p.number}</td><td><b>{p.name}</b></td><td>{p.functions.join(', ')}</td></tr>
              ))}
            </tbody>
          </table>
        </Section>
        <Section title="Interrupt matrix sources">
          <table className="grid-table info-table">
            <thead><tr><th>#</th><th>Name</th><th>Description</th></tr></thead>
            <tbody>
              {spec.interrupts.map((v) => (
                <tr key={v.source}><td>{v.source}</td><td><b>{v.name}</b></td><td>{v.desc}</td></tr>
              ))}
            </tbody>
          </table>
        </Section>
      </div>
    </div>
  );
}
