/**
 * Windows 7 style drop-down / context menus with icon gutter, check marks, shortcuts,
 * submenus and keyboard navigation.
 */
import { useEffect, useLayoutEffect, useRef, useState, type JSX, type ReactNode } from 'react';
import { Icons, type IconName } from '../icons';
import { COMMANDS, runCommand, shortcutLabel } from '../services/commands';

export type MenuItem =
  | { kind: 'cmd'; cmd: string; label?: string }
  | { kind: 'sep' }
  | { kind: 'sub'; label: string; icon?: IconName; items: () => MenuItem[]; disabled?: boolean }
  | { kind: 'action'; label: string; icon?: IconName; run: () => void; checked?: boolean; disabled?: boolean; shortcut?: string };

export const cmd = (id: string, label?: string): MenuItem => ({ kind: 'cmd', cmd: id, label });
export const sep: MenuItem = { kind: 'sep' };

interface Resolved {
  label: string;
  icon?: IconName;
  shortcut: string;
  disabled: boolean;
  checked?: boolean;
  sub?: () => MenuItem[];
  run?: () => void;
}

function resolve(item: MenuItem): Resolved | null {
  switch (item.kind) {
    case 'sep':
      return null;
    case 'cmd': {
      const c = COMMANDS[item.cmd];
      if (!c) return { label: item.cmd, shortcut: '', disabled: true };
      return {
        label: item.label ?? c.label,
        icon: c.icon,
        shortcut: shortcutLabel(c.keys),
        disabled: c.enabled ? !c.enabled() : false,
        checked: c.checked?.(),
        run: () => runCommand(c.id),
      };
    }
    case 'sub':
      return { label: item.label, icon: item.icon, shortcut: '', disabled: !!item.disabled, sub: item.items };
    case 'action':
      return { label: item.label, icon: item.icon, shortcut: item.shortcut ?? '', disabled: !!item.disabled, checked: item.checked, run: item.run };
  }
}

interface PopupProps {
  items: MenuItem[];
  x: number;
  y: number;
  onClose: () => void;
  /** For submenus: left edge to flip to when overflowing the right side. */
  flipX?: number;
  keyboard?: boolean;
  onLeft?: () => void;
  onRight?: () => void;
}

export function MenuPopup({ items, x, y, onClose, flipX, keyboard = true, onLeft, onRight }: PopupProps): JSX.Element {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });
  const [hot, setHot] = useState(-1);
  const [openSub, setOpenSub] = useState<{ index: number; x: number; y: number; flip: number } | null>(null);
  const resolved = items.map(resolve);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    let nx = x;
    let ny = y;
    if (nx + r.width > window.innerWidth - 2) nx = flipX !== undefined ? flipX - r.width : window.innerWidth - r.width - 2;
    if (ny + r.height > window.innerHeight - 2) ny = Math.max(2, window.innerHeight - r.height - 2);
    setPos({ x: Math.max(2, nx), y: ny });
  }, [x, y, flipX]);

  const activate = (i: number, el?: HTMLElement | null) => {
    const r = resolved[i];
    if (!r || r.disabled) return;
    if (r.sub) {
      const rect = (el ?? ref.current?.children[i] as HTMLElement | undefined)?.getBoundingClientRect();
      if (rect) setOpenSub({ index: i, x: rect.right - 2, y: rect.top - 3, flip: rect.left + 2 });
      return;
    }
    onClose();
    r.run?.();
  };

  useEffect(() => {
    if (!keyboard) return;
    const onKey = (e: KeyboardEvent) => {
      if (openSub) return;
      const n = resolved.length;
      const move = (d: number) => {
        let i = hot;
        for (let k = 0; k < n; k++) {
          i = (i + d + n) % n;
          if (resolved[i] && !resolved[i]!.disabled) break;
        }
        setHot(i);
      };
      if (e.key === 'ArrowDown') move(1);
      else if (e.key === 'ArrowUp') move(-1);
      else if (e.key === 'Enter' && hot >= 0) activate(hot);
      else if (e.key === 'ArrowRight' && hot >= 0 && resolved[hot]?.sub) activate(hot);
      else if (e.key === 'ArrowRight') onRight?.();
      else if (e.key === 'ArrowLeft') onLeft?.();
      else if (e.key === 'Escape') onClose();
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  });

  return (
    <div className="menu-popup" ref={ref} style={{ left: pos.x, top: pos.y }} onMouseDown={(e) => e.stopPropagation()}>
      {items.map((_item, i) => {
        const r = resolved[i];
        if (!r) return <div key={i} className="menu-separator" />;
        const Icon = r.icon ? Icons[r.icon] : null;
        return (
          <div
            key={i}
            className={`menu-entry${hot === i ? ' hot' : ''}${r.disabled ? ' disabled' : ''}`}
            onMouseEnter={(e) => {
              setHot(i);
              if (r.sub && !r.disabled) activate(i, e.currentTarget);
              else setOpenSub(null);
            }}
            onClick={(e) => activate(i, e.currentTarget)}
          >
            <span className="menu-icon">
              {r.checked !== undefined ? (
                <span className={`menu-check${r.checked ? ' on' : ''}`}>{r.checked && <CheckGlyph />}</span>
              ) : (
                Icon && <Icon />
              )}
            </span>
            <span className="menu-label">{r.label}</span>
            {r.shortcut && <span className="menu-shortcut">{r.shortcut}</span>}
            {r.sub && <span className="submenu-arrow" />}
          </div>
        );
      })}
      {openSub && resolved[openSub.index]?.sub && (
        <MenuPopup
          items={resolved[openSub.index]!.sub!()}
          x={openSub.x}
          y={openSub.y}
          flipX={openSub.flip}
          onClose={onClose}
          onLeft={() => setOpenSub(null)}
        />
      )}
    </div>
  );
}

function CheckGlyph(): JSX.Element {
  return (
    <svg width="10" height="9" viewBox="0 0 10 9">
      <path d="M1 4.6l2.6 2.6L9 1.4" stroke="#1c3d74" strokeWidth="1.7" fill="none" />
    </svg>
  );
}

/** Imperative context menu host. */
let showContext: ((items: MenuItem[], x: number, y: number) => void) | null = null;

export function openContextMenu(e: { clientX: number; clientY: number; preventDefault(): void }, items: MenuItem[]): void {
  e.preventDefault();
  showContext?.(items, e.clientX, e.clientY);
}

export function ContextMenuHost(): ReactNode {
  const [menu, setMenu] = useState<{ items: MenuItem[]; x: number; y: number } | null>(null);
  useEffect(() => {
    showContext = (items, x, y) => setMenu({ items, x, y });
    return () => {
      showContext = null;
    };
  }, []);
  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener('mousedown', close);
    window.addEventListener('blur', close);
    return () => {
      window.removeEventListener('mousedown', close);
      window.removeEventListener('blur', close);
    };
  }, [menu]);
  return menu ? <MenuPopup items={menu.items} x={menu.x} y={menu.y} onClose={() => setMenu(null)} /> : null;
}
