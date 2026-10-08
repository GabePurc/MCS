import type { JSX } from 'react';
import { useSim } from '../state/sim';
import { useWorkspace } from '../state/workspace';
import { useSettings } from '../state/settings';
import { formatHz, formatTime } from '../format';

export function StatusBar(): JSX.Element {
  const st = useSim((s) => s.state);
  const running = useSim((s) => s.running);
  const lastStop = useSim((s) => s.lastStop);
  const spec = useSim((s) => s.spec);
  const building = useWorkspace((s) => s.building);
  const cursor = useWorkspace((s) => s.cursor);
  const hasDoc = useWorkspace((s) => !!s.activeDocId);
  const speedMode = useSettings((s) => s.speedMode);

  let led = 'idle';
  let text = 'Ready';
  if (building) {
    led = 'pause';
    text = 'Building...';
  } else if (st?.resetHeld) {
    led = 'error';
    text = 'Held in reset (RESET pin low)';
  } else if (running) {
    led = 'run';
    text = st?.sleeping ? 'Running (CPU sleeping)' : 'Running';
  } else if (lastStop && st) {
    led = lastStop.reason === 'invalid' ? 'error' : 'pause';
    const where = `0x${(lastStop.pc * 2).toString(16).toUpperCase().padStart(4, '0')}`;
    text = {
      breakpoint: `Breakpoint hit at ${where}`,
      break: `BREAK at ${where}`,
      invalid: `Stopped: invalid opcode at ${where}`,
      step: `Paused at ${where}`,
      pause: `Paused at ${where}`,
      runTo: `Paused at ${where}`,
      reset: 'Reset - paused at 0x0000',
      load: 'Program loaded - paused at reset vector',
    }[lastStop.reason];
  }
  const ratio = st && running && st.speedHz > 0 ? st.speedHz / st.hz : 0;
  return (
    <div className="statusbar">
      <div className="status-cell grow">
        <span className={`status-led ${led}`} />
        {text}
      </div>
      {spec && (
        <div className="status-cell" data-tip="Target device and current CPU clock">
          {spec.name} @ {st ? formatHz(st.hz) : '-'}
        </div>
      )}
      {st && (
        <div className="status-cell mono" data-tip="Executed CPU cycles">
          {st.cycles.toLocaleString()} cyc
        </div>
      )}
      {st && (
        <div className="status-cell mono" data-tip="Simulated time since power-on">
          {formatTime(st.timeSec)}
        </div>
      )}
      <div className="status-cell" data-tip="Effective simulation speed">
        {running ? `${formatHz(st?.speedHz ?? 0)} (${ratio >= 10 ? ratio.toFixed(0) : ratio.toFixed(2)}x)` : speedMode === 'max' ? 'Max speed' : 'Real-time'}
      </div>
      {hasDoc && (
        <div className="status-cell">
          Ln {cursor.line} Col {cursor.col}
        </div>
      )}
      <div className="resize-grip" />
    </div>
  );
}
