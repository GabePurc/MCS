/**
 * Uncaught UI errors: written to the Output window (and the console) so a failure is visible
 * instead of silently breaking the window (GitHub issue #10).
 */
import { appendOutput } from '../state/workspace';

/** Browser noise that is not a fault of the app. */
const BENIGN = /ResizeObserver loop/;

/** Each distinct error is shown at most FLOOD_LIMIT times per FLOOD_WINDOW_MS. */
const FLOOD_LIMIT = 3;
const FLOOD_WINDOW_MS = 10_000;
const recent = new Map<string, { count: number; since: number }>();

/** Pop-out windows send their reports to the main window's Output (set by services/windows.ts). */
export const errorHooks: { remote: ((text: string) => void) | null } = { remote: null };

/** One-line description of a thrown value. */
export function describeError(e: unknown): string {
  if (e instanceof Error) return `${e.name}: ${e.message}`;
  return typeof e === 'string' ? e : JSON.stringify(e) ?? String(e);
}

/** Whether a report is worth showing (skips benign browser noise and floods of the same error). */
export function shouldReport(text: string, now = Date.now()): boolean {
  if (BENIGN.test(text)) return false;
  const r = recent.get(text);
  if (r && now - r.since < FLOOD_WINDOW_MS) return ++r.count <= FLOOD_LIMIT;
  if (recent.size >= 64) recent.clear();
  recent.set(text, { count: 1, since: now });
  return true;
}

export function reportError(where: string, e: unknown, componentStack?: string): void {
  const text = `Internal error in ${where}: ${describeError(e)}`;
  console.error(text, e, componentStack ?? '');
  if (!shouldReport(text)) return;
  const line = `${text}. Please report this on GitHub with what you were doing.`;
  try {
    if (errorHooks.remote) errorHooks.remote(line);
    else appendOutput('error', line);
  } catch {
    // The output store itself failed: the console line above is all we can do.
  }
}

let installed = false;

/** Reports errors that escape event handlers, timers and promises. */
export function installGlobalErrorHandlers(): void {
  if (installed) return;
  installed = true;
  window.addEventListener('error', (e) => reportError('the user interface', e.error ?? e.message));
  window.addEventListener('unhandledrejection', (e) => reportError('a background task', e.reason));
}
