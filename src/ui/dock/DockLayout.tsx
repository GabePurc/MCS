/**
 * Visual-Studio-2010-era docking: the editor in the centre, tool windows in four tab groups
 * (right top/bottom, bottom left/right) with resizable splitters. Tabs can be dragged between
 * groups (empty groups appear as drop targets while dragging) and closed; View menu restores.
 */
import { memo, useCallback, useEffect, useRef, useState, type CSSProperties, type JSX, type ReactNode } from 'react';
import { create } from 'zustand';
import { Icons } from '../icons';
import { useLayout, type PanelId, type ZoneId } from '../state/layout';
import { PANEL_ICONS, PANEL_TITLES } from '../services/commands';
import { useWorkspace } from '../state/workspace';
import { openContextMenu } from '../controls/Menu';

const useDrag = create<{ panel: PanelId | null }>(() => ({ panel: null }));

export function Splitter({ dir, onDrag, onEnd }: { dir: 'v' | 'h'; onDrag: (delta: number) => void; onEnd?: () => void }): JSX.Element {
  const [dragging, setDragging] = useState(false);
  return (
    <div
      className={`splitter ${dir}${dragging ? ' dragging' : ''}`}
      onPointerDown={(e) => {
        e.preventDefault();
        const el = e.currentTarget;
        el.setPointerCapture(e.pointerId);
        setDragging(true);
        let last = dir === 'v' ? e.clientX : e.clientY;
        let raf = 0;
        let pending = 0;
        const move = (ev: PointerEvent) => {
          const p = dir === 'v' ? ev.clientX : ev.clientY;
          pending += p - last;
          last = p;
          if (!raf) {
            raf = requestAnimationFrame(() => {
              raf = 0;
              onDrag(pending);
              pending = 0;
            });
          }
        };
        const up = () => {
          el.removeEventListener('pointermove', move);
          el.removeEventListener('pointerup', up);
          setDragging(false);
          onEnd?.();
        };
        el.addEventListener('pointermove', move);
        el.addEventListener('pointerup', up);
      }}
    />
  );
}

interface GroupProps {
  zone: ZoneId;
  style?: CSSProperties;
  render: (panel: PanelId) => ReactNode;
}

export const DockGroup = memo(function DockGroup({ zone, style, render }: GroupProps): JSX.Element | null {
  const z = useLayout((s) => s.zones[zone]);
  const dragging = useDrag((s) => s.panel);
  const [over, setOver] = useState(false);
  const empty = z.panels.length === 0;
  if (empty && !dragging) return null;

  const drop = (e: React.DragEvent) => {
    e.preventDefault();
    setOver(false);
    const p = e.dataTransfer.getData('text/mcs-panel') as PanelId;
    if (p) useLayout.getState().move(p, zone);
    useDrag.setState({ panel: null });
  };

  return (
    <div
      className={`dock-group${over ? ' drop-target' : ''}`}
      style={{ ...style, ...(empty ? { borderStyle: 'dashed', background: 'rgba(255,255,255,0.4)' } : {}) }}
      onDragOver={(e) => {
        if (!useDrag.getState().panel) return;
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={drop}
    >
      <div className="dock-tabs">
        {z.panels.map((p) => {
          const Icon = Icons[PANEL_ICONS[p]];
          return (
            <div
              key={p}
              className={`dock-tab${z.active === p ? ' active' : ''}`}
              draggable
              onDragStart={(e) => {
                e.dataTransfer.setData('text/mcs-panel', p);
                e.dataTransfer.effectAllowed = 'move';
                useDrag.setState({ panel: p });
              }}
              onDragEnd={() => useDrag.setState({ panel: null })}
              onMouseDown={() => useLayout.getState().setActive(zone, p)}
              onContextMenu={(e) =>
                openContextMenu(e, [
                  { kind: 'action', label: 'Close', run: () => useLayout.getState().close(p) },
                  { kind: 'sep' },
                  ...(['rightTop', 'rightBottom', 'bottomLeft', 'bottomRight'] as ZoneId[])
                    .filter((t) => t !== zone)
                    .map((t) => ({ kind: 'action' as const, label: `Move to ${ZONE_LABEL[t]}`, run: () => useLayout.getState().move(p, t) })),
                ])
              }
            >
              <Icon size={14} />
              {PANEL_TITLES[p]}
              <span
                className="tab-close"
                data-tip="Close"
                onMouseDown={(e) => e.stopPropagation()}
                onClick={() => useLayout.getState().close(p)}
              >
                <Icons.Close size={7} />
              </span>
            </div>
          );
        })}
        <div className="tabs-spacer" />
      </div>
      <div className="dock-body">{empty ? <div className="dock-empty">Drop a window here</div> : z.active && render(z.active)}</div>
    </div>
  );
});

const ZONE_LABEL: Record<ZoneId, string> = { rightTop: 'Right (top)', rightBottom: 'Right (bottom)', bottomLeft: 'Bottom (left)', bottomRight: 'Bottom (right)' };

export function DockLayout({ editor, render }: { editor: ReactNode; render: (p: PanelId) => ReactNode }): JSX.Element {
  const zones = useLayout((s) => s.zones);
  const rightWidth = useLayout((s) => s.rightWidth);
  const bottomHeight = useLayout((s) => s.bottomHeight);
  const rightSplit = useLayout((s) => s.rightSplit);
  const bottomSplit = useLayout((s) => s.bottomSplit);
  const dragging = useDrag((s) => s.panel);
  const showOutputSeq = useWorkspace((s) => s.showOutputSeq);
  const mainRef = useRef<HTMLDivElement>(null);
  const rightRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (showOutputSeq > 0) useLayout.getState().show('output');
  }, [showOutputSeq]);

  const has = (z: ZoneId) => zones[z].panels.length > 0 || !!dragging;
  const right = has('rightTop') || has('rightBottom');
  const bottom = has('bottomLeft') || has('bottomRight');
  const set = useLayout.getState().setSize;

  const dragRight = useCallback((d: number) => {
    const max = (mainRef.current?.clientWidth ?? 1200) - 300;
    set({ rightWidth: Math.max(220, Math.min(max, useLayout.getState().rightWidth - d)) });
  }, [set]);
  const dragBottom = useCallback((d: number) => {
    const max = (mainRef.current?.clientHeight ?? 800) - 120;
    set({ bottomHeight: Math.max(90, Math.min(max, useLayout.getState().bottomHeight - d)) });
  }, [set]);
  const dragRightSplit = useCallback((d: number) => {
    const h = rightRef.current?.clientHeight ?? 600;
    set({ rightSplit: Math.max(0.12, Math.min(0.88, useLayout.getState().rightSplit + d / h)) });
  }, [set]);
  const dragBottomSplit = useCallback((d: number) => {
    const w = bottomRef.current?.clientWidth ?? 800;
    set({ bottomSplit: Math.max(0.15, Math.min(0.85, useLayout.getState().bottomSplit + d / w)) });
  }, [set]);

  return (
    <div className="main-area" ref={mainRef}>
      <div className="dock-col" style={{ flex: 1 }}>
        <div className="dock-col" style={{ flex: 1, minHeight: 80 }}>{editor}</div>
        {bottom && <Splitter dir="h" onDrag={dragBottom} />}
        {bottom && (
          <div className="dock-row" style={{ height: bottomHeight, flex: 'none' }} ref={bottomRef}>
            <DockGroup zone="bottomLeft" render={render} style={{ flex: has('bottomRight') ? bottomSplit : 1 }} />
            {has('bottomLeft') && has('bottomRight') && <Splitter dir="v" onDrag={dragBottomSplit} />}
            <DockGroup zone="bottomRight" render={render} style={{ flex: has('bottomLeft') ? 1 - bottomSplit : 1 }} />
          </div>
        )}
      </div>
      {right && <Splitter dir="v" onDrag={dragRight} />}
      {right && (
        <div className="dock-col" style={{ width: rightWidth, flex: 'none' }} ref={rightRef}>
          <DockGroup zone="rightTop" render={render} style={{ flex: has('rightBottom') ? rightSplit : 1 }} />
          {has('rightTop') && has('rightBottom') && <Splitter dir="h" onDrag={dragRightSplit} />}
          <DockGroup zone="rightBottom" render={render} style={{ flex: has('rightTop') ? 1 - rightSplit : 1 }} />
        </div>
      )}
    </div>
  );
}
