/**
 * UI-side proxy for the Rust simulation thread. Commands go out through `sim_command`; state
 * snapshots stream back over a Tauri channel and are merged into the stores (panels subscribe
 * to slices of them).
 */
import { simAttach, simCommand } from '../backend/api';
import type { MachineState, RawMachineState, SimCommand, SimOutput } from '../backend/types';
import { useSim } from '../state/sim';
import { trace } from '../state/trace';
import { appendOutput } from '../state/workspace';

let attached: Promise<void> | null = null;
const queue: SimCommand[] = [];

/** Connects to the backend once; commands issued before the connection is ready are queued. */
export function connectSim(): Promise<void> {
  if (!attached) {
    attached = simAttach(onOutput)
      .then(() => {
        for (const c of queue.splice(0)) simCommand(c);
      })
      .catch((e) => {
        appendOutput('error', `Simulator unavailable: ${e instanceof Error ? e.message : String(e)}`);
      });
  }
  return attached;
}

export function sim(cmd: SimCommand): void {
  if (!attached) {
    queue.push(cmd);
    void connectSim();
    return;
  }
  void attached.then(() => simCommand(cmd));
}

function onOutput(o: SimOutput): void {
  switch (o.type) {
    case 'device':
      useSim.setState({ spec: o.spec, baseline: null, lastStopped: null });
      return;
    case 'error':
      appendOutput('error', o.message);
      useSim.setState({ error: o.message });
      return;
    case 'state':
      applyState(convert(o.state));
  }
}

function convert(r: RawMachineState): MachineState {
  return {
    ...r,
    regs: Uint8Array.from(r.regs),
    data: Uint8Array.from(r.data),
    flash: r.flash ? Uint8Array.from(r.flash) : undefined,
    traceCycles: Float64Array.from(r.traceCycles),
    traceLevels: Uint32Array.from(r.traceLevels),
  };
}

function applyState(st: MachineState): void {
  const cur = useSim.getState();
  for (const m of st.messages) appendOutput(m.level === 'warning' ? 'warning' : m.level === 'error' ? 'error' : 'info', `[sim @ ${m.cycle}] ${m.text}`);
  const patch: Partial<typeof cur> = { state: st, running: st.running };
  if (st.flash) patch.flash = st.flash;
  if (st.stop?.reason === 'load' || (st.stop?.reason === 'reset' && st.cycles === 0)) {
    trace.clear();
  }
  trace.append(st.traceCycles, st.traceLevels, st.cycles, st.hz);
  if (st.stop) {
    patch.lastStop = st.stop;
    patch.revealSeq = cur.revealSeq + 1;
    if (st.stop.reason === 'load') {
      patch.baseline = st;
      patch.lastStopped = st;
      patch.stopwatch = { cycles: 0, time: 0 };
    } else {
      patch.baseline = cur.lastStopped ?? st;
      patch.lastStopped = st;
    }
    if (st.stop.message) appendOutput(st.stop.reason === 'invalid' ? 'error' : 'info', `${st.stop.message} at 0x${(st.stop.pc * 2).toString(16).toUpperCase().padStart(4, '0')}`);
  }
  useSim.setState(patch);
}
