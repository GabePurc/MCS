/**
 * Window layout: tool panels live in four dock groups around the editor (right top/bottom,
 * bottom left/right), float above the main window, or open in their own OS window ("pop out").
 * Panels can be closed, re-opened from the View menu and dragged between groups; sizes,
 * placement and floating rectangles persist across sessions (popped-out windows do not).
 */
import { create } from 'zustand';
import { loadJson, saveJson } from './persist';

export type PanelId =
  | 'processor' | 'io' | 'memory' | 'disasm' | 'pins' | 'wave' | 'output' | 'symbols' | 'callstack' | 'breakpoints'
  | 'chip' | 'info' | 'isa' | 'serial';
export type ZoneId = 'rightTop' | 'rightBottom' | 'bottomLeft' | 'bottomRight';

export const ZONES: ZoneId[] = ['rightTop', 'rightBottom', 'bottomLeft', 'bottomRight'];

export interface Zone {
  panels: PanelId[];
  active: PanelId | null;
}

export interface FloatRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface LayoutData {
  zones: Record<ZoneId, Zone>;
  /** Dock group a panel returns to. */
  home: Record<PanelId, ZoneId>;
  /** Panels that re-open floating instead of docked. */
  floatHome: PanelId[];
  /** Open floating windows, bottom to top. */
  floating: PanelId[];
  /** Last floating rectangle per panel. */
  floatRects: Partial<Record<PanelId, FloatRect>>;
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
    chip: 'bottomLeft', info: 'rightTop', isa: 'rightBottom', serial: 'bottomLeft',
  },
  floatHome: ['chip', 'info', 'isa'],
  floating: [],
  floatRects: {
    chip: { x: 120, y: 90, w: 820, h: 600 },
    info: { x: 180, y: 110, w: 640, h: 620 },
    isa: { x: 240, y: 130, w: 600, h: 520 },
  },
  rightWidth: 380,
  bottomHeight: 250,
  rightSplit: 0.55,
  bottomSplit: 0.58,
};

const KEY = 'mcs.layout.v1';

/** Hooks installed by the window manager (pop-out windows live outside this store). */
export const windowHooks = {
  /** Opens `panel` in its own OS window; returns false when that is not possible. */
  popOut: (_panel: PanelId): boolean => false,
  /** Brings an existing pop-out window to front. */
  focusPopout: (_panel: PanelId): void => {},
  /** Closes a pop-out window. */
  closePopout: (_panel: PanelId): void => {},
  /** In a pop-out window: forwards show requests to the main window. */
  remoteShow: null as ((panel: PanelId) => void) | null,
};

interface LayoutStore extends LayoutData {
  /** Panels currently shown in their own OS window (not persisted). */
  popped: PanelId[];
  show(panel: PanelId): void;
  close(panel: PanelId): void;
  toggle(panel: PanelId): void;
  setActive(zone: ZoneId, panel: PanelId): void;
  move(panel: PanelId, zone: ZoneId, index?: number): void;
  /** Detaches a panel into a floating window. */
  float(panel: PanelId, rect?: Partial<FloatRect>): void;
  setFloatRect(panel: PanelId, rect: FloatRect): void;
  raise(panel: PanelId): void;
  /** Docks a floating / popped-out panel into its home group. */
  dock(panel: PanelId): void;
  popOut(panel: PanelId): void;
  /** A pop-out window went away: `dock` re-docks the panel, otherwise it is closed. */
  popoutClosed(panel: PanelId, dock: boolean): void;
  setSize(patch: Partial<Pick<LayoutData, 'rightWidth' | 'bottomHeight' | 'rightSplit' | 'bottomSplit'>>): void;
  reset(): void;
}

function clone(d: LayoutData): LayoutData {
  return JSON.parse(JSON.stringify(d));
}

function load(): LayoutData {
  const d = loadJson<LayoutData>(KEY, clone(DEFAULT));
  // Validate: every panel at most once across zones and floating windows.
  const seen = new Set<string>();
  for (const z of ZONES) {
    if (!d.zones[z]) d.zones[z] = { panels: [], active: null };
    d.zones[z].panels = d.zones[z].panels.filter((p) => p in DEFAULT.home && !seen.has(p) && seen.add(p));
    if (d.zones[z].active && !d.zones[z].panels.includes(d.zones[z].active!)) d.zones[z].active = d.zones[z].panels[0] ?? null;
  }
  d.home = { ...DEFAULT.home, ...d.home };
  d.floatHome = Array.isArray(d.floatHome) ? d.floatHome.filter((p) => p in DEFAULT.home) : [...DEFAULT.floatHome];
  d.floating = (Array.isArray(d.floating) ? d.floating : []).filter((p) => p in DEFAULT.home && !seen.has(p) && seen.add(p));
  d.floatRects = { ...DEFAULT.floatRects, ...d.floatRects };
  return d;
}

/** Keeps a floating window reachable inside the main window. */
export function clampRect(r: FloatRect): FloatRect {
  const W = typeof window === 'undefined' ? 1440 : window.innerWidth;
  const H = typeof window === 'undefined' ? 900 : window.innerHeight;
  const w = Math.max(240, Math.min(r.w, W - 20));
  const h = Math.max(140, Math.min(r.h, H - 40));
  return { w, h, x: Math.max(-w + 120, Math.min(r.x, W - 120)), y: Math.max(0, Math.min(r.y, H - 60)) };
}

export const useLayout = create<LayoutStore>((set, get) => {
  const save = () => {
    const { zones, home, floatHome, floating, floatRects, rightWidth, bottomHeight, rightSplit, bottomSplit } = get();
    saveJson(KEY, { zones, home, floatHome, floating, floatRects, rightWidth, bottomHeight, rightSplit, bottomSplit });
  };
  const zoneOf = (p: PanelId): ZoneId | null => ZONES.find((z) => get().zones[z].panels.includes(p)) ?? null;
  /** Removes a panel from its dock group / floating list (state patch). */
  const detach = (s: LayoutStore, panel: PanelId): Partial<LayoutStore> => {
    const zones = { ...s.zones };
    const z = ZONES.find((id) => zones[id].panels.includes(panel));
    if (z) {
      const panels = zones[z].panels.filter((p) => p !== panel);
      const active = zones[z].active === panel ? panels[Math.max(0, zones[z].panels.indexOf(panel) - 1)] ?? null : zones[z].active;
      zones[z] = { panels, active };
    }
    return { zones, floating: s.floating.filter((p) => p !== panel), popped: s.popped.filter((p) => p !== panel), home: z ? { ...s.home, [panel]: z } : s.home };
  };
  const dockInto = (panel: PanelId, zone: ZoneId, index?: number) =>
    set((s) => {
      const base = detach(s, panel);
      const zones = base.zones!;
      const target = zones[zone].panels.slice();
      target.splice(index ?? target.length, 0, panel);
      zones[zone] = { panels: target, active: panel };
      return { ...base, zones, home: { ...base.home!, [panel]: zone }, floatHome: s.floatHome.filter((p) => p !== panel) };
    });
  return {
    ...load(),
    popped: [],
    show(panel) {
      if (windowHooks.remoteShow) {
        windowHooks.remoteShow(panel);
        return;
      }
      const s = get();
      const z = zoneOf(panel);
      if (z) {
        s.setActive(z, panel);
        return;
      }
      if (s.floating.includes(panel)) {
        s.raise(panel);
        return;
      }
      if (s.popped.includes(panel)) {
        windowHooks.focusPopout(panel);
        return;
      }
      if (s.floatHome.includes(panel)) s.float(panel);
      else dockInto(panel, s.home[panel]);
      save();
    },
    close(panel) {
      if (get().popped.includes(panel)) windowHooks.closePopout(panel);
      set((s) => detach(s, panel));
      save();
    },
    toggle(panel) {
      const s = get();
      if (zoneOf(panel) || s.floating.includes(panel) || s.popped.includes(panel)) s.close(panel);
      else s.show(panel);
    },
    setActive(zone, panel) {
      if (get().zones[zone].active === panel) return;
      set((s) => ({ zones: { ...s.zones, [zone]: { ...s.zones[zone], active: panel } } }));
      save();
    },
    move(panel, zone, index) {
      dockInto(panel, zone, index);
      save();
    },
    float(panel, rect) {
      const s = get();
      const prev = s.floatRects[panel] ?? { x: 160 + s.floating.length * 24, y: 110 + s.floating.length * 24, w: 520, h: 420 };
      const r = clampRect({ ...prev, ...rect });
      if (s.popped.includes(panel)) windowHooks.closePopout(panel);
      set((st) => {
        const base = detach(st, panel);
        return {
          ...base,
          floating: [...base.floating!, panel],
          floatRects: { ...st.floatRects, [panel]: r },
          floatHome: st.floatHome.includes(panel) ? st.floatHome : [...st.floatHome, panel],
        };
      });
      save();
    },
    setFloatRect(panel, rect) {
      set((s) => ({ floatRects: { ...s.floatRects, [panel]: clampRect(rect) } }));
      save();
    },
    raise(panel) {
      const f = get().floating;
      if (f[f.length - 1] === panel || !f.includes(panel)) return;
      set({ floating: [...f.filter((p) => p !== panel), panel] });
    },
    dock(panel) {
      const s = get();
      if (s.popped.includes(panel)) windowHooks.closePopout(panel);
      dockInto(panel, s.home[panel]);
      save();
    },
    popOut(panel) {
      if (get().popped.includes(panel)) {
        windowHooks.focusPopout(panel);
        return;
      }
      if (!windowHooks.popOut(panel)) return;
      set((s) => {
        const base = detach(s, panel);
        return { ...base, popped: [...base.popped!, panel] };
      });
      save();
    },
    popoutClosed(panel, dock) {
      if (!get().popped.includes(panel)) return;
      set((s) => ({ popped: s.popped.filter((p) => p !== panel) }));
      if (dock) get().dock(panel);
    },
    setSize(patch) {
      set(patch);
      save();
    },
    reset() {
      for (const p of get().popped) windowHooks.closePopout(p);
      set({ ...clone(DEFAULT), popped: [] });
      save();
    },
  };
});

export function isPanelOpen(panel: PanelId): boolean {
  const s = useLayout.getState();
  return ZONES.some((id) => s.zones[id].panels.includes(panel)) || s.floating.includes(panel) || s.popped.includes(panel);
}
