import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource/cascadia-mono/400.css';
import './styles/win7.css';
import './styles/app.css';
import './styles/panels.css';
import { App } from './App';
import { PopoutApp } from './PopoutApp';
import { popoutPanel } from './services/windows';
import { ErrorBoundary, showFatalFallback } from './controls/ErrorBoundary';
import { installGlobalErrorHandlers, reportError } from './services/errors';

installGlobalErrorHandlers();

// Suppress the browser context menu outside of editable fields (the app provides its own).
window.addEventListener('contextmenu', (e) => {
  const t = e.target as HTMLElement;
  if (!t.closest('input, textarea, .cm-content, .selectable')) e.preventDefault();
});

createRoot(document.getElementById('root')!, {
  onUncaughtError: (error) => {
    reportError('MCS', error);
    showFatalFallback();
  },
}).render(
  <StrictMode>
    <ErrorBoundary name="MCS" window>
      {popoutPanel ? <PopoutApp panel={popoutPanel} /> : <App />}
    </ErrorBoundary>
  </StrictMode>,
);
