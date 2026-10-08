import type { JSX } from 'react';
import { Icons } from '../icons';
import { COMMANDS, runCommand, setSpeed, shortcutLabel, speedLabel, SPEEDS } from '../services/commands';
import type { SpeedMode } from '../backend/types';
import { openDialog } from '../state/dialogs';
import { formatHz } from '../format';
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


export function Toolbar(): JSX.Element {
  // Subscribe to everything command enablement depends on.
  useSim((s) => s.running);
  useWorkspace((s) => [s.activeDocId, s.building, !!s.build, s.docs.length].join());
  const speedMode = useSettings((s) => s.speedMode);
  const speedFactor = useSettings((s) => s.speedFactor);
  const deviceId = useSettings((s) => s.deviceId);
  const devices = useDevices((s) => s.devices);
  const mcuHz = useSim((s) => s.state?.hz ?? 0);
  const speedValue = speedMode === 'max' ? 'max:1' : `${speedMode}:${speedFactor}`;
  const options: [string, string][] = SPEEDS.map(([, f, m]) => [`${m}:${f}`, m === 'realtime' && f === 1 ? `Real-time${mcuHz ? ` (${formatHz(mcuHz)})` : ''}` : speedLabel(m, f)]);
  if (!options.some(([v]) => v === speedValue)) options.push([speedValue, speedLabel(speedMode, speedFactor)]);
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
        data-tip={'Simulation speed: a fixed CPU clock from 1 Hz (watch every instruction),\nthe real MCU speed (or a multiple of it), or as fast as possible'}
        onChange={(e) => {
          if (e.target.value === 'custom') {
            openDialog('speed');
            return;
          }
          const [m, f] = e.target.value.split(':');
          setSpeed(m as SpeedMode, Number(f));
        }}
      >
        {options.map(([v, l]) => (
          <option key={v} value={v}>{l}</option>
        ))}
        <option value="custom">Custom...</option>
      </select>
      <span className="tb-label">Device:</span>
      <select className="w7-select" value={deviceId} data-tip="Target microcontroller" onChange={(e) => selectDevice(e.target.value)}>
        {[...new Set(devices.map((d) => d.family))].map((fam) => (
          <optgroup key={fam} label={fam}>
            {devices.filter((d) => d.family === fam).map((d) => (
              <option key={d.id} value={d.id}>{d.name}</option>
            ))}
          </optgroup>
        ))}
      </select>
    </div>
  );
}
