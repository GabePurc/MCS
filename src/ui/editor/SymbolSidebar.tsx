import { useState, type JSX } from 'react';
import { editorApi } from './editorApi';
import { useSymbolOutline } from './symbolView';

const IDENT = /^[A-Za-z_]\w*$/;

/** Symbol View sidebar: the document's labels / functions in file order, plus "add symbol". */
export function SymbolSidebar(): JSX.Element {
  const { kind, items, top, active } = useSymbolOutline();
  const [adding, setAdding] = useState<string | null>(null);
  const noun = kind === 'c' ? 'function' : 'label';
  const err = adding === null || adding === '' ? null
    : !IDENT.test(adding) ? 'Use letters, digits and _ (not starting with a digit).'
    : items.some((s) => s.name.toLowerCase() === adding.toLowerCase()) ? `"${adding}" already exists.` : null;
  const commit = () => {
    if (adding && !err) editorApi()?.addSymbol(adding);
    setAdding(null);
  };
  return (
    <div className="symview-sidebar">
      <div className="symview-head">
        <span>{kind === 'c' ? 'Functions' : 'Symbols'}</span>
        {kind !== 'none' && (
          <button className="symview-add" data-tip={`Add a ${noun} after the shown one`} onClick={() => setAdding('')}>
            +
          </button>
        )}
      </div>
      <div className="symview-list">
        {top && (
          <div className={`symview-item top${active === -1 ? ' active' : ''}`} onClick={() => editorApi()?.focusSymbol(-1)}>
            <span>{items.length ? '(top of file)' : '(whole file)'}</span>
          </div>
        )}
        {items.map((s, i) => (
          <div key={i} className={`symview-item${active === i ? ' active' : ''}`} onClick={() => editorApi()?.focusSymbol(i)} data-tip={`Line ${s.line}`}>
            <span className="mono">{s.name}</span>
            <span className="line">{s.line}</span>
          </div>
        ))}
        {!items.length && kind !== 'none' && adding === null && <div className="symview-empty">No {noun}s yet.</div>}
        {adding !== null && (
          <div className="symview-new">
            <input
              className="w7-input mono"
              autoFocus
              value={adding}
              placeholder={`New ${noun} name`}
              onChange={(e) => setAdding(e.target.value.trim())}
              onKeyDown={(e) => {
                if (e.key === 'Enter') commit();
                else if (e.key === 'Escape') setAdding(null);
              }}
              onBlur={() => setAdding(null)}
            />
            {err && <div className="error-text">{err}</div>}
          </div>
        )}
      </div>
    </div>
  );
}
