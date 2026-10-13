/**
 * Uncaught UI errors: written to the Output window (and the console) so a failure is visible
 * instead of silently breaking the window (GitHub issue #10).
 */
import { appendOutput } from '../state/workspace';

/** Browser noise that is not a fault of the app. */
const BENIGN = /ResizeObserver loop/;

let lastText = '';
let repeats = 0;

/** One-line description of a thrown value. */
export function describeError(e: unknown): string {
  if (e instanceof Error) return `${e.name}: ${e.message}`;
  return typeof e === 'string' ? e : JSON.stringify(e) ?? String(e);
}

/** Whether a report is worth showing (skips benign browser noise and floods of the same error). */
export function shouldReport(text: string): boolean {
  if (BENIGN.test(text)) return false;
  if (text === lastText) return ++repeats < 3;
  lastText = text;
  repeats = 0;
  return true;
}

export function reportError(where: string, e: unknown, componentStack?: string): void {
  const text = `Internal error in ${where}: ${describeError(e)}`;
  console.error(text, e, componentStack ?? '');
  if (!shouldReport(text)) return;
  try {
    appendOutput('error', `${text}. Please report this on GitHub with what you were doing.`);
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
