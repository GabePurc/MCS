import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource/cascadia-mono/400.css';
import './styles/win7.css';
import './styles/app.css';
import './styles/panels.css';
import { App } from './App';
import { PopoutApp } from './PopoutApp';
import { popoutPanel } from './services/windows';

// Suppress the browser context menu outside of editable fields (the app provides its own).
window.addEventListener('contextmenu', (e) => {
  const t = e.target as HTMLElement;
  if (!t.closest('input, textarea, .cm-content, .selectable')) e.preventDefault();
});

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {popoutPanel ? <PopoutApp panel={popoutPanel} /> : <App />}
  </StrictMode>,
);
