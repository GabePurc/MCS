import type { JSX } from 'react';
import { Icons } from '../icons';
import { COMMANDS, runCommand, setSpeed, shortcutLabel } from '../services/commands';
import { useSim } from '../state/sim';
import { useSettings } from '../state/settings';
import { useWorkspace } from '../state/workspace';
import { useDevices } from '../state/devices';
import { selectDevice } from '../services/device';

/** Toolbar button bound to a command (re-renders with the stores it depends on). */
export function CmdButton({ id, label }: { id: string; label?: boolean }): JSX.Element {
  const c = COMMANDS[id];
  const Icon = c.icon ? Icons[c.icon] : null;
  const enabled = c.enabled ? c.enabled() : true;
  const sc = shortcutLabel(c.keys);
  return (
    <button className="tb-btn" disabled={!enabled} data-tip={`${c.label}${sc ? ` (${sc})` : ''}`} onClick={() => runCommand(id)}>
      {Icon && <Icon />}
      {label && <span>{c.label}</span>}
    </button>
  );
}

const SPEED_OPTIONS: [string, string][] = [
  ['realtime:0.01', '1/100x'],
  ['realtime:0.1', '1/10x'],
  ['realtime:1', 'Real-time'],
  ['realtime:10', '10x'],
  ['max:1', 'Maximum'],
];

export function Toolbar(): JSX.Element {
  // Subscribe to everything command enablement depends on.
  useSim((s) => s.running);
  useWorkspace((s) => [s.activeDocId, s.building, !!s.build, s.docs.length].join());
  const speedMode = useSettings((s) => s.speedMode);
  const speedFactor = useSettings((s) => s.speedFactor);
  const deviceId = useSettings((s) => s.deviceId);
  const devices = useDevices((s) => s.devices);
  const speedValue = speedMode === 'max' ? 'max:1' : `realtime:${speedFactor}`;
  return (
    <div className="toolbar">
      <div className="tb-grip" />
      <CmdButton id="file.newAsm" />
      <CmdButton id="file.open" />
      <CmdButton id="file.save" />
      <CmdButton id="file.import" />
      <div className="tb-sep" />
      <CmdButton id="build.build" label />
      <div className="tb-sep" />
      <CmdButton id="debug.start" />
      <CmdButton id="debug.pause" />
      <CmdButton id="debug.stop" />
      <CmdButton id="debug.reset" />
      <div className="tb-sep" />
      <CmdButton id="debug.stepInto" />
      <CmdButton id="debug.stepOver" />
      <CmdButton id="debug.stepOut" />
      <CmdButton id="debug.runToCursor" />
      <div className="tb-sep" />
      <CmdButton id="debug.toggleBreakpoint" />
      <CmdButton id="debug.clearBreakpoints" />
      <div className="tb-sep" />
      <span className="tb-label">Speed:</span>
      <select
        className="w7-select"
        value={speedValue}
        data-tip="Simulation speed relative to the MCU's real clock"
        onChange={(e) => {
          const [m, f] = e.target.value.split(':');
          setSpeed(m as 'realtime' | 'max', Number(f));
        }}
      >
        {SPEED_OPTIONS.map(([v, l]) => (
          <option key={v} value={v}>{l}</option>
        ))}
      </select>
      <span className="tb-label">Device:</span>
      <select className="w7-select" value={deviceId} data-tip="Target microcontroller" onChange={(e) => selectDevice(e.target.value)}>
        {devices.map((d) => (
          <option key={d.id} value={d.id}>{d.name}</option>
        ))}
      </select>
    </div>
  );
}
