/**
 * Dock layout: tool panels live in four dock groups around the editor (right top/bottom,
 * bottom left/right). Panels can be closed, re-opened from the View menu and dragged between
 * groups; sizes and placement persist across sessions.
 */
import { create } from 'zustand';
import { loadJson, saveJson } from './persist';

export type PanelId = 'processor' | 'io' | 'memory' | 'disasm' | 'pins' | 'wave' | 'output' | 'symbols' | 'callstack' | 'breakpoints';
export type ZoneId = 'rightTop' | 'rightBottom' | 'bottomLeft' | 'bottomRight';

export const ZONES: ZoneId[] = ['rightTop', 'rightBottom', 'bottomLeft', 'bottomRight'];

export interface Zone {
  panels: PanelId[];
  active: PanelId | null;
}

export interface LayoutData {
  zones: Record<ZoneId, Zone>;
  home: Record<PanelId, ZoneId>;
  rightWidth: number;
  bottomHeight: number;
  /** Fraction of the right column taken by the top group. */
  rightSplit: number;
  /** Fraction of the bottom row taken by the left group. */
  bottomSplit: number;
}

const DEFAULT: LayoutData = {
  zones: {
    rightTop: { panels: ['processor', 'io'], active: 'processor' },
    rightBottom: { panels: ['pins', 'symbols'], active: 'pins' },
    bottomLeft: { panels: ['output', 'wave', 'memory'], active: 'output' },
    bottomRight: { panels: ['disasm', 'callstack', 'breakpoints'], active: 'disasm' },
  },
  home: {
    processor: 'rightTop', io: 'rightTop', pins: 'rightBottom', symbols: 'rightBottom',
    output: 'bottomLeft', wave: 'bottomLeft', memory: 'bottomLeft', disasm: 'bottomRight', callstack: 'bottomRight', breakpoints: 'bottomRight',
  },
  rightWidth: 380,
  bottomHeight: 250,
  rightSplit: 0.55,
  bottomSplit: 0.58,
};

const KEY = 'mcs.layout.v1';

interface LayoutStore extends LayoutData {
  show(panel: PanelId): void;
  close(panel: PanelId): void;
  toggle(panel: PanelId): void;
  setActive(zone: ZoneId, panel: PanelId): void;
  move(panel: PanelId, zone: ZoneId, index?: number): void;
  setSize(patch: Partial<Pick<LayoutData, 'rightWidth' | 'bottomHeight' | 'rightSplit' | 'bottomSplit'>>): void;
  reset(): void;
}

function clone(d: LayoutData): LayoutData {
  return JSON.parse(JSON.stringify(d));
}

function load(): LayoutData {
  const d = loadJson<LayoutData>(KEY, clone(DEFAULT));
  // Validate: every panel exactly once in zones or hidden.
  const seen = new Set<string>();
  for (const z of ZONES) {
    if (!d.zones[z]) d.zones[z] = { panels: [], active: null };
    d.zones[z].panels = d.zones[z].panels.filter((p) => p in DEFAULT.home && !seen.has(p) && seen.add(p));
    if (d.zones[z].active && !d.zones[z].panels.includes(d.zones[z].active!)) d.zones[z].active = d.zones[z].panels[0] ?? null;
  }
  d.home = { ...DEFAULT.home, ...d.home };
  return d;
}

export const useLayout = create<LayoutStore>((set, get) => {
  const save = () => {
    const { zones, home, rightWidth, bottomHeight, rightSplit, bottomSplit } = get();
    saveJson(KEY, { zones, home, rightWidth, bottomHeight, rightSplit, bottomSplit });
  };
  const zoneOf = (p: PanelId): ZoneId | null => ZONES.find((z) => get().zones[z].panels.includes(p)) ?? null;
  return {
    ...load(),
    show(panel) {
      const z = zoneOf(panel);
      if (z) {
        get().setActive(z, panel);
        return;
      }
      const home = get().home[panel];
      set((s) => ({ zones: { ...s.zones, [home]: { panels: [...s.zones[home].panels, panel], active: panel } } }));
      save();
    },
    close(panel) {
      const z = zoneOf(panel);
      if (!z) return;
      set((s) => {
        const panels = s.zones[z].panels.filter((p) => p !== panel);
        const active = s.zones[z].active === panel ? panels[Math.max(0, s.zones[z].panels.indexOf(panel) - 1)] ?? null : s.zones[z].active;
        return { zones: { ...s.zones, [z]: { panels, active } }, home: { ...s.home, [panel]: z } };
      });
      save();
    },
    toggle(panel) {
      if (zoneOf(panel)) get().close(panel);
      else get().show(panel);
    },
    setActive(zone, panel) {
      if (get().zones[zone].active === panel) return;
      set((s) => ({ zones: { ...s.zones, [zone]: { ...s.zones[zone], active: panel } } }));
      save();
    },
    move(panel, zone, index) {
      const from = zoneOf(panel);
      set((s) => {
        const zones = { ...s.zones };
        if (from) {
          const panels = zones[from].panels.filter((p) => p !== panel);
          zones[from] = { panels, active: zones[from].active === panel ? panels[0] ?? null : zones[from].active };
        }
        const target = zones[zone].panels.filter((p) => p !== panel);
        target.splice(index ?? target.length, 0, panel);
        zones[zone] = { panels: target, active: panel };
        return { zones, home: { ...s.home, [panel]: zone } };
      });
      save();
    },
    setSize(patch) {
      set(patch);
      save();
    },
    reset() {
      set(clone(DEFAULT));
      save();
    },
  };
});

export function isPanelOpen(panel: PanelId): boolean {
  const z = useLayout.getState().zones;
  return ZONES.some((id) => z[id].panels.includes(panel));
}
