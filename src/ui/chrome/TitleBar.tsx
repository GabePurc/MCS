import { useEffect, type JSX } from 'react';
import { CaptionGlyph, Icons } from '../icons';
import { inTauri, win } from '../backend/api';
import { useWorkspace } from '../state/workspace';
import { runCommand } from '../services/commands';

/** Aero glass caption with Windows 7 caption buttons. Drag to move, double-click to maximize. */
export function TitleBar({ maximized }: { maximized: boolean }): JSX.Element {
  const title = useWorkspace((s) => {
    const d = s.docs.find((x) => x.id === s.activeDocId);
    return d ? `${d.name}${d.dirty ? ' *' : ''} - MCS Microcontroller Simulator` : 'MCS Microcontroller Simulator';
  });
  useEffect(() => {
    document.title = title;
  }, [title]);
  return (
    <div
      className="titlebar"
      onMouseDown={(e) => {
        if (e.button !== 0 || (e.target as HTMLElement).closest('.caption-buttons')) return;
        if (e.detail === 2) win.toggleMaximize();
        else if (e.detail === 1) win.startDragging();
      }}
    >
      <Icons.App className="titlebar-icon" />
      <div className="titlebar-title">{title}</div>
      {inTauri && (
        <div className="caption-buttons">
          <button className="caption-btn" title="Minimize" onClick={() => win.minimize()}>
            <CaptionGlyph.Min />
          </button>
          <button className="caption-btn" title={maximized ? 'Restore Down' : 'Maximize'} onClick={() => win.toggleMaximize()}>
            {maximized ? <CaptionGlyph.Restore /> : <CaptionGlyph.Max />}
          </button>
          <button className="caption-btn close" title="Close" onClick={() => runCommand('file.exit')}>
            <CaptionGlyph.Close />
          </button>
        </div>
      )}
    </div>
  );
}
