import { create } from 'zustand';
import { loadJson, saveJson } from './persist';
import type { SpeedMode } from '../backend/types';

export interface Settings {
  deviceId: string;
  /** Explicit avr-gcc path ('' = auto-detect). */
  gccPath: string;
  /** C optimisation level flag without dash, e.g. "Os". */
  optimize: string;
  extraFlags: string;
  recentFiles: string[];
  editorFontSize: number;
  speedMode: SpeedMode;
  speedFactor: number;
  /** Re-open these files at start-up. */
  openFiles: string[];
  showStartPage: boolean;
  /** Step by source line (when line info exists) instead of by instruction. */
  sourceStepping: boolean;
  vcc: number;
}

const DEFAULTS: Settings = {
  deviceId: 'attiny10',
  gccPath: '',
  optimize: 'Os',
  extraFlags: '',
  recentFiles: [],
  editorFontSize: 13,
  speedMode: 'realtime',
  speedFactor: 1,
  openFiles: [],
  showStartPage: true,
  sourceStepping: true,
  vcc: 5,
};

const KEY = 'mcs.settings.v1';

interface SettingsStore extends Settings {
  set(patch: Partial<Settings>): void;
  addRecent(path: string): void;
}

export const useSettings = create<SettingsStore>((set, get) => ({
  ...loadJson(KEY, DEFAULTS),
  set(patch) {
    set(patch);
    persist(get());
  },
  addRecent(path) {
    const recent = [path, ...get().recentFiles.filter((p) => p !== path)].slice(0, 10);
    set({ recentFiles: recent });
    persist(get());
  },
}));

function persist(s: SettingsStore): void {
  const out: Settings = {
    deviceId: s.deviceId, gccPath: s.gccPath, optimize: s.optimize, extraFlags: s.extraFlags, recentFiles: s.recentFiles,
    editorFontSize: s.editorFontSize, speedMode: s.speedMode, speedFactor: s.speedFactor, openFiles: s.openFiles, showStartPage: s.showStartPage,
    sourceStepping: s.sourceStepping, vcc: s.vcc,
  };
  saveJson(KEY, out);
}
