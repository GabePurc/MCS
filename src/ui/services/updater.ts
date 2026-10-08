/**
 * In-app updates (desktop app): checks the latest GitHub release's `latest.json`, downloads the
 * signed installer for this platform, installs it and restarts. Signatures are verified by the
 * Tauri updater plugin against the public key in tauri.conf.json.
 */
import { create } from 'zustand';
import { inTauri } from '../backend/api';
import { openDialog } from '../state/dialogs';
import { useSettings } from '../state/settings';
import { appendOutput } from '../state/workspace';
import { confirmQuit } from './files';

export const APP_VERSION = __APP_VERSION__;

interface PendingUpdate {
  version: string;
  date?: string;
  notes: string;
  downloadAndInstall(onEvent: (e: { event: string; data?: { contentLength?: number; chunkLength?: number } }) => void): Promise<void>;
}

export type UpdateState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'none'; checkedAt: number }
  | { kind: 'available'; version: string; date?: string; notes: string }
  | { kind: 'downloading'; version: string; done: number; total: number }
  | { kind: 'installed'; version: string }
  | { kind: 'error'; message: string };

export const useUpdates = create<{ state: UpdateState }>(() => ({ state: { kind: 'idle' } }));

let pending: PendingUpdate | null = null;

const set = (state: UpdateState) => useUpdates.setState({ state });

/** Release notes without the download/installation instructions that follow them. */
function cleanNotes(body: string | undefined): string {
  return (body ?? '').split(/\n#+\s*Download\b/i)[0].replace(/^#+\s*What's new\s*\n/i, '').trim();
}

/** Looks for a newer release. `manual` shows the result even when there is no update. */
export async function checkForUpdates(manual: boolean): Promise<void> {
  if (!inTauri) return;
  const cur = useUpdates.getState().state.kind;
  if (cur === 'checking' || cur === 'downloading') return;
  set({ kind: 'checking' });
  try {
    const { check } = await import('@tauri-apps/plugin-updater');
    const u = await check();
    if (!u) {
      pending = null;
      set({ kind: 'none', checkedAt: Date.now() });
      if (manual) appendOutput('info', `MCS ${APP_VERSION} is the latest version.`);
      return;
    }
    pending = { version: u.version, date: u.date, notes: cleanNotes(u.body), downloadAndInstall: (cb) => u.downloadAndInstall(cb) };
    set({ kind: 'available', version: u.version, date: u.date, notes: pending.notes });
    if (!manual) appendOutput('info', `MCS ${u.version} is available (you have ${APP_VERSION}). Help > Check for Updates installs it.`);
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    set({ kind: 'error', message });
    if (manual) appendOutput('warning', `Update check failed: ${message}`);
  }
}

/** Downloads and installs the pending update, then restarts the app. */
export async function installUpdate(): Promise<void> {
  const u = pending;
  if (!u) return;
  // Installing restarts the app: offer to save open documents first.
  if (!(await confirmQuit())) return;
  let done = 0;
  let total = 0;
  set({ kind: 'downloading', version: u.version, done: 0, total: 0 });
  try {
    await u.downloadAndInstall((e) => {
      if (e.event === 'Started') total = e.data?.contentLength ?? 0;
      if (e.event === 'Progress') done += e.data?.chunkLength ?? 0;
      set({ kind: 'downloading', version: u.version, done, total });
    });
    set({ kind: 'installed', version: u.version });
    const { relaunch } = await import('@tauri-apps/plugin-process');
    await relaunch();
  } catch (e) {
    set({ kind: 'error', message: e instanceof Error ? e.message : String(e) });
  }
}

/** Background check a few seconds after start-up (desktop app, when enabled). */
export function scheduleStartupCheck(): void {
  if (!inTauri || !useSettings.getState().autoUpdateCheck) return;
  setTimeout(() => void checkForUpdates(false), 5000);
}

export function openUpdateDialog(): void {
  openDialog('update');
  const k = useUpdates.getState().state.kind;
  if (k === 'idle' || k === 'none' || k === 'error') void checkForUpdates(true);
}
