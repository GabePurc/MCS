import { useSettings } from '../state/settings';
import { useWorkspace, appendOutput } from '../state/workspace';
import { sim } from './simClient';
import { loadProgram } from './build';

/** Switches the target device; reloads the current program image (if any) on the new part. */
export function selectDevice(id: string): void {
  if (useSettings.getState().deviceId === id) return;
  useSettings.getState().set({ deviceId: id });
  const b = useWorkspace.getState().build;
  appendOutput('info', `Target device: ${id}${b ? ' (rebuild to re-check the program for this device)' : ''}`);
  if (b) loadProgram(b.program, b.docId, b.label, id);
  else sim({ type: 'init', deviceId: id });
}
