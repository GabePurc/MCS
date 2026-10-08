/** Small building blocks shared by the debug panels. */
import { useState, type JSX, type ReactNode } from 'react';
import { parseNumber } from '../format';

/** Double-click-to-edit value cell (hex/dec/bin input accepted). */
export function EditableValue({ value, display, onCommit, className, title, max = 0xff }: {
  value: number;
  display: string;
  onCommit: (v: number) => void;
  className?: string;
  title?: string;
  max?: number;
}): JSX.Element {
  const [editing, setEditing] = useState(false);
  if (editing) {
    return (
      <input
        className="inline-edit mono"
        autoFocus
        defaultValue={display}
        onFocus={(e) => e.currentTarget.select()}
        onBlur={() => setEditing(false)}
        onKeyDown={(e) => {
          if (e.key === 'Escape') setEditing(false);
          if (e.key === 'Enter') {
            const v = parseNumber(e.currentTarget.value);
            if (!Number.isNaN(v) && v >= 0 && v <= max) onCommit(v);
            setEditing(false);
          }
        }}
      />
    );
  }
  return (
    <span className={`editable mono ${className ?? ""}`} data-value={value} data-tip={title ?? "Double-click to edit"} onDoubleClick={() => setEditing(true)}>
      {display}
    </span>
  );
}

export function Section({ title, children, right }: { title: string; children?: ReactNode; right?: ReactNode }): JSX.Element {
  return (
    <>
      <div className="section-header">
        {title}
        <span className="line" />
        {right}
      </div>
      {children}
    </>
  );
}

export function EmptyHint({ children }: { children: ReactNode }): JSX.Element {
  return <div className="empty-hint">{children}</div>;
}
