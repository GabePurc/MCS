/**
 * Error boundaries (GitHub issue #10). Without them an exception while rendering unmounts the
 * whole React tree, leaving a blank window without caption buttons that cannot even be closed.
 * A crashed panel or editor now shows an error box in its place; if the window itself fails it
 * falls back to a minimal window that can still save, reload and close.
 */
import { Component, useEffect, type ErrorInfo, type JSX, type ReactNode } from 'react';
import { Icons } from '../icons';
import { Caption, useMaximized } from '../chrome/TitleBar';
import { inTauri, win } from '../backend/api';
import { confirmQuit, saveAll } from '../services/files';
import { describeError, reportError } from '../services/errors';
import { leavePopout, popoutPanel } from '../services/windows';
import { useWorkspace } from '../state/workspace';

interface Props {
  /** Shown in the message ("<name> stopped working"). */
  name: string;
  children: ReactNode;
  /** Root of a window: the fallback replaces the whole window. */
  window?: boolean;
  /** A change clears the error (e.g. switching to another document). */
  resetKey?: unknown;
}

interface State {
  error: unknown;
  componentStack: string;
}

export class ErrorBoundary extends Component<Props, State> {
  override state: State = { error: null, componentStack: '' };

  static getDerivedStateFromError(error: unknown): Partial<State> {
    return { error: error ?? new Error('Unknown error') };
  }

  override componentDidCatch(error: unknown, info: ErrorInfo): void {
    this.setState({ componentStack: info.componentStack ?? '' });
    reportError(this.props.name, error, info.componentStack ?? undefined);
  }

  override componentDidUpdate(prev: Props): void {
    if (this.state.error && prev.resetKey !== this.props.resetKey) this.reset();
  }

  reset = (): void => this.setState({ error: null, componentStack: '' });

  override render(): ReactNode {
    const { error, componentStack } = this.state;
    if (!error) return this.props.children;
    if (this.props.window) return <CrashedWindow error={error} componentStack={componentStack} />;
    return (
      <div className="crash-box">
        <b>{this.props.name} stopped working</b>
        <div className="crash-msg selectable">{describeError(error)}</div>
        <button className="w7-btn small" onClick={this.reset}>
          <span>Try Again</span>
        </button>
      </div>
    );
  }
}

function details(error: unknown, componentStack: string): string {
  const stack = error instanceof Error && error.stack ? error.stack : describeError(error);
  return `${stack}\n\nComponents:${componentStack}`;
}

/** Closes the crashed window: the main window asks about unsaved files first. */
async function closeWindow(): Promise<void> {
  if (popoutPanel) leavePopout(false);
  else if (!(await confirmQuit())) return;
  win.destroy();
}

function CrashedWindow({ error, componentStack }: { error: unknown; componentStack: string }): JSX.Element {
  const maximized = useMaximized();
  // The fallback must not fail itself: the workspace may be what broke.
  const dirty = useWorkspace((s) => {
    try {
      return !popoutPanel && s.docs.some((d) => d.dirty);
    } catch {
      return false;
    }
  });
  // The window's close guard went away with the crashed tree: Alt+F4 / taskbar close still asks.
  useEffect(() => {
    const unsub = win.onCloseRequested(async () => {
      if (popoutPanel) leavePopout(false);
      return popoutPanel ? true : confirmQuit();
    });
    return () => void unsub.then((f) => f());
  }, []);
  return (
    <div className={`app crashed${maximized ? ' maximized' : ''}`}>
      <Caption icon={<Icons.App className="titlebar-icon" />} title="MCS Microcontroller Simulator" maximized={maximized} onClose={() => void closeWindow()} />
      <div className="crash-page">
        <h1>MCS ran into a problem</h1>
        <p>
          The window stopped working because of an internal error. {dirty ? 'Your unsaved files are still in memory: save them before reloading. ' : ''}
          Reloading restores the window and reopens your saved files.
        </p>
        <pre className="crash-details selectable">{details(error, componentStack)}</pre>
        <div className="crash-actions">
          {dirty && (
            <button className="w7-btn" onClick={() => void saveAll()}>
              <span>Save All</span>
            </button>
          )}
          <button className="w7-btn default" onClick={() => window.location.reload()}>
            <span>Reload Window</span>
          </button>
          <button className="w7-btn" onClick={() => void navigator.clipboard.writeText(details(error, componentStack))}>
            <span>Copy Details</span>
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * Last resort when even the window fallback fails (React then unmounts everything): plain DOM
 * that can still reload or close the window.
 */
export function showFatalFallback(): void {
  if (document.getElementById('fatal-error')) return;
  const page = document.createElement('div');
  page.id = 'fatal-error';
  page.className = 'crash-page fatal';
  page.innerHTML =
    '<h1>MCS ran into a problem</h1><p>The window stopped working because of an internal error.</p>' +
    '<div class="crash-actions"><button class="w7-btn default"><span>Reload Window</span></button></div>';
  const actions = page.querySelector('.crash-actions')!;
  actions.querySelector('button')!.onclick = () => window.location.reload();
  if (inTauri) {
    const close = document.createElement('button');
    close.className = 'w7-btn';
    close.innerHTML = '<span>Close</span>';
    close.onclick = () => win.destroy();
    actions.appendChild(close);
  }
  document.body.appendChild(page);
}
