import { useEffect, useRef, useState, type JSX } from 'react';
import { clearOutput, requestGoto, useWorkspace, type OutputLevel } from '../state/workspace';
import { openPath } from '../services/files';
import { sameFile } from '../services/debugInfo';
import { Icons } from '../icons';

const ICON: Record<OutputLevel, JSX.Element | null> = {
  info: null,
  cmd: null,
  success: <Icons.Success size={13} />,
  warning: <Icons.Warning size={13} />,
  error: <Icons.Error size={13} />,
};

/** Build output and simulator messages; double-click a diagnostic to jump to the source. */
export function OutputPanel(): JSX.Element {
  const lines = useWorkspace((s) => s.output);
  const [filter, setFilter] = useState<'all' | 'problems'>('all');
  const ref = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  useEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);
  const shown = filter === 'all' ? lines : lines.filter((l) => l.level === 'error' || l.level === 'warning');
  const errors = lines.filter((l) => l.level === 'error').length;
  const warnings = lines.filter((l) => l.level === 'warning').length;
  const jump = async (file?: string, line?: number) => {
    if (!file || !line) return;
    const ws = useWorkspace.getState();
    const doc = ws.docs.find((d) => sameFile(file, d.path ?? d.name));
    if (doc) requestGoto(doc.id, line);
    else if (await openPath(file)) requestGoto(useWorkspace.getState().activeDocId!, line);
  };
  return (
    <div className="panel">
      <div className="panel-toolbar">
        <span>Show:</span>
        <select className="w7-select" value={filter} onChange={(e) => setFilter(e.target.value as 'all' | 'problems')}>
          <option value="all">All output</option>
          <option value="problems">Errors and warnings</option>
        </select>
        <span className="out-count"><Icons.Error size={13} /> {errors}</span>
        <span className="out-count"><Icons.Warning size={13} /> {warnings}</span>
        <button className="tb-btn" style={{ marginLeft: 'auto' }} data-tip="Clear all" onClick={clearOutput}>
          <Icons.Clear />
        </button>
      </div>
      <div
        className="panel-scroll output-text mono"
        ref={ref}
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 4;
        }}
      >
        {shown.map((l) => (
          <div key={l.id} className={`out-line out-${l.level}${l.file ? ' link' : ''}`} onDoubleClick={() => void jump(l.file, l.line)} data-tip={l.file ? 'Double-click to go to the source line' : undefined}>
            <span className="out-icon">{ICON[l.level]}</span>
            {l.text}
          </div>
        ))}
      </div>
    </div>
  );
}
