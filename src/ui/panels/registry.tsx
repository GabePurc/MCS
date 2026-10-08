/** Tool panel registry shared by the main window, floating windows and pop-out windows. */
import { lazy, Suspense, type JSX } from 'react';
import type { PanelId } from '../state/layout';
import { ProcessorPanel } from './ProcessorPanel';
import { IoViewPanel } from './IoViewPanel';
import { MemoryPanel } from './MemoryPanel';
import { DisassemblyPanel } from './DisassemblyPanel';
import { PinsPanel } from './PinsPanel';
import { WaveformPanel } from './WaveformPanel';
import { OutputPanel } from './OutputPanel';
import { SymbolsPanel } from './SymbolsPanel';
import { CallStackPanel } from './CallStackPanel';
import { BreakpointsPanel } from './BreakpointsPanel';
import { DeviceInfoPanel } from './DeviceInfoPanel';
import { IsaPanel } from './IsaPanel';
import { EmptyHint } from './common';

// The 3D view pulls in three.js: load it only when the panel is first shown.
const ChipView = lazy(() => import('../chip/ChipView').then((m) => ({ default: m.ChipView })));

function ChipPanel(): JSX.Element {
  return (
    <Suspense fallback={<EmptyHint>Loading the 3D chip view...</EmptyHint>}>
      <ChipView />
    </Suspense>
  );
}

const PANELS: Record<PanelId, () => JSX.Element> = {
  processor: ProcessorPanel,
  io: IoViewPanel,
  memory: MemoryPanel,
  disasm: DisassemblyPanel,
  pins: PinsPanel,
  wave: WaveformPanel,
  output: OutputPanel,
  symbols: SymbolsPanel,
  callstack: CallStackPanel,
  breakpoints: BreakpointsPanel,
  chip: ChipPanel,
  info: DeviceInfoPanel,
  isa: IsaPanel,
};

export function renderPanel(id: PanelId): JSX.Element {
  const P = PANELS[id];
  return <P />;
}
