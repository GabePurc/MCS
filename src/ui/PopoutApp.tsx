/**
 * Root of a pop-out window: one tool panel in its own OS window, mirroring the main window's
 * simulator and workspace state (see services/windows.ts).
 */
import { useEffect, useState, type JSX } from 'react';
import { IconDefs, Icons } from './icons';
import { Caption } from './chrome/TitleBar';
import { ContextMenuHost } from './controls/Menu';
import { TooltipHost } from './controls/Tooltip';
import { renderPanel } from './panels/registry';
import { handleShortcut, PANEL_ICONS, PANEL_TITLES } from './services/commands';
import { initPopoutBridge, leavePopout } from './services/windows';
import { inTauri, win } from './backend/api';
import { EmptyHint } from './panels/common';
import type { PanelId } from './state/layout';
import { startSerial } from './state/serial';

let started = false;

export function PopoutApp({ panel }: { panel: PanelId }): JSX.Element {
  const [ready, setReady] = useState(false);
  const [maximized, setMaximized] = useState(false);
  const [focused, setFocused] = useState(true);
  const Icon = Icons[PANEL_ICONS[panel]];
  const title = `${PANEL_TITLES[panel]} - MCS`;

  useEffect(() => {
    document.title = title;
    if (started) return;
    started = true;
    void initPopoutBridge(panel, () => (inTauri ? win.destroy() : window.close())).then(() => {
      startSerial();
      setReady(true);
    });
  }, [panel, title]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => handleShortcut(e);
    window.addEventListener('keydown', onKey);
    const unsubs = [
      win.onResized(() => void win.isMaximized().then(setMaximized)),
      win.onFocusChanged(setFocused),
      win.onCloseRequested(async () => {
        leavePopout(false);
        return true;
      }),
    ];
    return () => {
      window.removeEventListener('keydown', onKey);
      unsubs.forEach((u) => void u.then((f) => f()));
    };
  }, []);

  const dock = () => {
    leavePopout(true);
    if (inTauri) win.destroy();
    else window.close();
  };

  return (
    <div className={`app popout${maximized ? ' maximized' : ''}${focused ? '' : ' inactive'}`}>
      <IconDefs />
      <Caption
        className="popout-titlebar"
        icon={<Icon className="titlebar-icon" />}
        title={title}
        maximized={maximized}
        onClose={() => {
          leavePopout(false);
          win.destroy();
        }}
      >
        <button className="w7-btn small popout-dock" data-tip="Put this window back into the main window" onClick={dock}>
          <Icons.Dock size={13} /> <span>Dock</span>
        </button>
      </Caption>
      <div className="popout-body dock-group">
        <div className="dock-body">{ready ? renderPanel(panel) : <EmptyHint>Connecting to the MCS main window...</EmptyHint>}</div>
      </div>
      <ContextMenuHost />
      <TooltipHost />
    </div>
  );
}
