/**
 * Floating tool windows above the main window (Visual Studio style "Float"). Drag the caption
 * to move, the edges/corner to resize; dropping the caption on a dock group's tab strip docks
 * the panel there, double-clicking the caption docks it back home.
 */
import { memo, useRef, useState, type JSX, type PointerEvent as RPointerEvent, type ReactNode } from 'react';
import { create } from 'zustand';
import { Icons } from '../icons';
import { clampRect, useLayout, type FloatRect, type PanelId, type ZoneId } from '../state/layout';
import { PANEL_ICONS, PANEL_TITLES } from '../services/commands';

/** Dock group currently targeted while a floating window is dragged. */
export const useDockHover = create<{ zone: ZoneId | null }>(() => ({ zone: null }));

function zoneAt(x: number, y: number): ZoneId | null {
  for (const el of document.elementsFromPoint(x, y)) {
    if (el.closest('.float-win')) continue;
    const tabs = el.closest('.dock-tabs');
    const group = tabs?.closest('[data-zone]') as HTMLElement | null;
    if (group) return group.dataset.zone as ZoneId;
  }
  return null;
}

type Mode = 'move' | 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw';

const FloatingWindow = memo(function FloatingWindow({ panel, z, top, render }: { panel: PanelId; z: number; top: boolean; render: (p: PanelId) => ReactNode }): JSX.Element {
  const rect = useLayout((s) => s.floatRects[panel]) ?? { x: 160, y: 120, w: 520, h: 420 };
  const [live, setLive] = useState<FloatRect | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const r = live ?? rect;
  const Icon = Icons[PANEL_ICONS[panel]];

  const start = (mode: Mode) => (e: RPointerEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest('button')) return;
    e.preventDefault();
    e.stopPropagation();
    useLayout.getState().raise(panel);
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const x0 = e.clientX;
    const y0 = e.clientY;
    const base = { ...rect };
    let cur = base;
    let raf = 0;
    const move = (ev: PointerEvent) => {
      const dx = ev.clientX - x0;
      const dy = ev.clientY - y0;
      const n = { ...base };
      if (mode === 'move') {
        n.x += dx;
        n.y += dy;
      } else {
        if (mode.includes('e')) n.w = Math.max(240, base.w + dx);
        if (mode.includes('s')) n.h = Math.max(140, base.h + dy);
        if (mode.includes('w')) {
          n.w = Math.max(240, base.w - dx);
          n.x = base.x + base.w - n.w;
        }
        if (mode.includes('n')) {
          n.h = Math.max(140, base.h - dy);
          n.y = base.y + base.h - n.h;
        }
      }
      cur = n;
      if (!raf) {
        raf = requestAnimationFrame(() => {
          raf = 0;
          setLive(cur);
          if (mode === 'move') {
            const zone = zoneAt(ev.clientX, ev.clientY);
            if (useDockHover.getState().zone !== zone) useDockHover.setState({ zone });
          }
        });
      }
    };
    const up = (ev: PointerEvent) => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
      cancelAnimationFrame(raf);
      useDockHover.setState({ zone: null });
      setLive(null);
      const zone = mode === 'move' ? zoneAt(ev.clientX, ev.clientY) : null;
      if (zone) useLayout.getState().move(panel, zone);
      else useLayout.getState().setFloatRect(panel, clampRect(cur));
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
  };

  return (
    <div
      ref={ref}
      className={`float-win${top ? ' active' : ''}`}
      style={{ left: r.x, top: r.y, width: r.w, height: r.h, zIndex: 40 + z }}
      onPointerDownCapture={() => useLayout.getState().raise(panel)}
    >
      <div className="float-title" onPointerDown={start('move')} onDoubleClick={() => useLayout.getState().dock(panel)}>
        <Icon size={14} />
        <span className="grow">{PANEL_TITLES[panel]}</span>
        <button className="float-btn" data-tip="Dock (double-click the caption, or drop it on a tab strip)" onClick={() => useLayout.getState().dock(panel)}>
          <Icons.Dock size={13} />
        </button>
        <button className="float-btn" data-tip="Open in a new window" onClick={() => useLayout.getState().popOut(panel)}>
          <Icons.PopOut size={13} />
        </button>
        <button className="float-btn close" data-tip="Close" onClick={() => useLayout.getState().close(panel)}>
          <Icons.Close size={8} />
        </button>
      </div>
      <div className="float-body dock-body">{render(panel)}</div>
      {(['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw'] as Mode[]).map((m) => (
        <div key={m} className={`float-edge ${m}`} onPointerDown={start(m)} />
      ))}
    </div>
  );
});

export function FloatingWindows({ render }: { render: (p: PanelId) => ReactNode }): JSX.Element {
  const floating = useLayout((s) => s.floating);
  return (
    <>
      {floating.map((p, i) => (
        <FloatingWindow key={p} panel={p} z={i} top={i === floating.length - 1} render={render} />
      ))}
    </>
  );
}
