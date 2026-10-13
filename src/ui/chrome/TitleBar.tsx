import { useEffect, useState, type JSX, type ReactNode } from 'react';
import { CaptionGlyph, Icons } from '../icons';
import { inTauri, win } from '../backend/api';
import { useWorkspace } from '../state/workspace';
import { runCommand } from '../services/commands';

/** Whether the window is maximized (follows resizes). */
export function useMaximized(): boolean {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    const unsub = win.onResized(() => void win.isMaximized().then(setMaximized));
    void win.isMaximized().then(setMaximized);
    return () => void unsub.then((f) => f());
  }, []);
  return maximized;
}

/**
 * Aero glass caption with Windows 7 caption buttons (desktop only). Drag to move, double-click
 * to maximize. `children` sit between the title and the caption buttons.
 */
export function Caption({ icon, title, maximized, onClose, className, children }: {
  icon: ReactNode;
  title: string;
  maximized: boolean;
  onClose: () => void;
  className?: string;
  children?: ReactNode;
}): JSX.Element {
  return (
    <div
      className={className ? `titlebar ${className}` : 'titlebar'}
      onMouseDown={(e) => {
        if (e.button !== 0 || (e.target as HTMLElement).closest('button, .caption-buttons')) return;
        if (e.detail === 2) win.toggleMaximize();
        else if (e.detail === 1) win.startDragging();
      }}
    >
      {icon}
      <div className="titlebar-title">{title}</div>
      {children}
      {inTauri && (
        <div className="caption-buttons">
          <button className="caption-btn" title="Minimize" onClick={() => win.minimize()}>
            <CaptionGlyph.Min />
          </button>
          <button className="caption-btn" title={maximized ? 'Restore Down' : 'Maximize'} onClick={() => win.toggleMaximize()}>
            {maximized ? <CaptionGlyph.Restore /> : <CaptionGlyph.Max />}
          </button>
          <button className="caption-btn close" title="Close" onClick={onClose}>
            <CaptionGlyph.Close />
          </button>
        </div>
      )}
    </div>
  );
}

/** Main window caption: the active document's name. */
export function TitleBar({ maximized }: { maximized: boolean }): JSX.Element {
  const title = useWorkspace((s) => {
    const d = s.docs.find((x) => x.id === s.activeDocId);
    return d ? `${d.name}${d.dirty ? ' *' : ''} - MCS Microcontroller Simulator` : 'MCS Microcontroller Simulator';
  });
  useEffect(() => {
    document.title = title;
  }, [title]);
  return <Caption icon={<Icons.App className="titlebar-icon" />} title={title} maximized={maximized} onClose={() => runCommand('file.exit')} />;
}
