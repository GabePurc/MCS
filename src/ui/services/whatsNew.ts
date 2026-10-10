/**
 * "What's New" after an update: the changelog sections between the version that ran last on this
 * computer and the running one, shown once at start-up (and on demand from the Help menu).
 * The changelog is CHANGELOG.md, bundled at build time.
 */
import changelog from '../../../CHANGELOG.md?raw';
import { openDialog } from '../state/dialogs';
import { APP_VERSION } from './updater';

export interface ChangelogSection {
  version: string;
  body: string;
}

const LAST_KEY = 'mcs.lastVersion';
/** Present when the app ran before; versions up to 0.2.0 did not record `LAST_KEY`. */
const SETTINGS_KEY = 'mcs.settings.v1';

/** Released sections (`## [x.y.z]`), newest first; `[Unreleased]` is skipped. */
export function parseChangelog(md: string): ChangelogSection[] {
  const out: ChangelogSection[] = [];
  let cur: ChangelogSection | null = null;
  for (const line of md.split(/\r?\n/)) {
    const h = /^## \[([^\]]+)\]/.exec(line);
    if (h) {
      cur = /^\d+(\.\d+)*$/.test(h[1]) ? { version: h[1], body: '' } : null;
      if (cur) out.push(cur);
    } else if (cur) {
      cur.body += line + '\n';
    }
  }
  for (const s of out) s.body = s.body.trim();
  return out;
}

/** Numeric dotted-version comparison (`0.10.0` > `0.9.1`). */
export function compareVersions(a: string, b: string): number {
  const pa = a.split('.').map(Number);
  const pb = b.split('.').map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d) return d;
  }
  return 0;
}

/**
 * Sections to show at start-up, or none. `last` is the version recorded by the previous run
 * (`null` when nothing was recorded); `usedBefore` tells an update from a version that did not
 * record it apart from a fresh install (which shows nothing).
 */
export function sectionsToShow(sections: ChangelogSection[], current: string, last: string | null, usedBefore: boolean): ChangelogSection[] {
  if (last === null) return usedBefore ? sections.filter((s) => s.version === current) : [];
  if (compareVersions(last, current) >= 0) return [];
  return sections.filter((s) => compareVersions(s.version, last) > 0 && compareVersions(s.version, current) <= 0);
}

let shown: ChangelogSection[] = [];

/** Sections the What's New dialog displays. */
export function whatsNewSections(): ChangelogSection[] {
  return shown;
}

/** Start-up: opens What's New once after an update and records the running version. */
export function showWhatsNewAfterUpdate(): void {
  let last: string | null = null;
  let usedBefore = false;
  try {
    last = localStorage.getItem(LAST_KEY);
    usedBefore = localStorage.getItem(SETTINGS_KEY) !== null;
    localStorage.setItem(LAST_KEY, APP_VERSION);
  } catch {
    return; // no storage: we could not tell whether it was already shown
  }
  const s = sectionsToShow(parseChangelog(changelog), APP_VERSION, last, usedBefore);
  if (!s.length) return;
  shown = s;
  openDialog('whatsNew');
}

/** Help > What's New: the running version's notes. */
export function openWhatsNew(): void {
  const all = parseChangelog(changelog);
  shown = all.filter((s) => s.version === APP_VERSION);
  if (!shown.length) shown = all.slice(0, 1);
  openDialog('whatsNew');
}
