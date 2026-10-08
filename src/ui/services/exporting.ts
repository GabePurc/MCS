import { exportHex, pickSavePath } from '../backend/api';
import { appendOutput, useWorkspace } from '../state/workspace';

/** File > Export Intel HEX: writes the current program image. */
export async function exportHexDialog(): Promise<void> {
  const b = useWorkspace.getState().build;
  if (!b) return;
  const name = b.label.replace(/\.[^.]+$/, '') + '.hex';
  const path = await pickSavePath('Export Intel HEX', name, [{ name: 'Intel HEX', extensions: ['hex'] }]);
  if (!path) return;
  try {
    await exportHex(path, b.program.flash, b.program.flashUsed);
    appendOutput('success', `Exported ${b.program.flashUsed} bytes to ${path}`);
  } catch (e) {
    appendOutput('error', `Export failed: ${e instanceof Error ? e.message : String(e)}`);
  }
}
