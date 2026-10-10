import { useDeferredValue, useEffect, useMemo, useState, type JSX } from 'react';
import { defInclude } from '../backend/api';
import { useSim } from '../state/sim';
import { EmptyHint } from './common';

/** The generated avrasm2 definitions include (`tn10def.inc`...) as a tool window, with a line filter. */
export function DefinitionsPanel(): JSX.Element {
  const deviceId = useSim((s) => (s.spec?.arch === 'avr' ? s.spec.id : undefined));
  const arm = useSim((s) => s.spec?.arch === 'arm');
  const [inc, setInc] = useState<[string, string] | null>(null);
  const [filter, setFilter] = useState('');
  const query = useDeferredValue(filter.trim().toLowerCase());
  useEffect(() => {
    setInc(null);
    if (deviceId) defInclude(deviceId).then(setInc).catch(() => {});
  }, [deviceId]);
  const text = useMemo(() => {
    if (!inc || !query) return inc?.[1] ?? '';
    return inc[1].split('\n').filter((l) => l.toLowerCase().includes(query)).join('\n');
  }, [inc, query]);
  if (arm) return <EmptyHint>Device definition files (.inc) are available for AVR devices.</EmptyHint>;
  if (!deviceId) return <EmptyHint>No device selected.</EmptyHint>;
  if (!inc) return <EmptyHint>Loading...</EmptyHint>;
  return (
    <div className="defs-panel">
      <div className="panel-toolbar">
        <span className="mono" data-tip={`Use .include "${inc[0]}" in assembly sources`}>.include "{inc[0]}"</span>
        <input className="w7-input" value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter lines (e.g. PORTB, TCCR0)" />
        <button className="w7-btn" onClick={() => void navigator.clipboard.writeText(inc[1])}><span>Copy</span></button>
      </div>
      <pre className="mono selectable defs-view">{text || '(no matching lines)'}</pre>
    </div>
  );
}
