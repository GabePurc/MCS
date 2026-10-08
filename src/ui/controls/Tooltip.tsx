/**
 * Windows 7 style tooltips for any element with a `data-tip` attribute (shown after a short
 * hover delay, hidden on mouse down). One global host keeps per-element cost at zero.
 */
import { useEffect, useState, type JSX } from 'react';

export function TooltipHost(): JSX.Element | null {
  const [tip, setTip] = useState<{ text: string; x: number; y: number } | null>(null);
  useEffect(() => {
    let timer = 0;
    let current: HTMLElement | null = null;
    const over = (e: MouseEvent) => {
      const el = (e.target as HTMLElement | null)?.closest?.('[data-tip]') as HTMLElement | null;
      if (el === current) return;
      current = el;
      window.clearTimeout(timer);
      setTip(null);
      if (!el) return;
      timer = window.setTimeout(() => {
        const text = el.dataset.tip;
        if (!text || !el.isConnected) return;
        const r = el.getBoundingClientRect();
        const x = Math.min(e.clientX + 2, window.innerWidth - 300);
        setTip({ text, x: Math.max(4, x), y: Math.min(r.bottom + 4, window.innerHeight - 40) });
      }, 550);
    };
    const hide = () => {
      window.clearTimeout(timer);
      setTip(null);
    };
    window.addEventListener('mouseover', over);
    window.addEventListener('mousedown', hide, true);
    window.addEventListener('wheel', hide, true);
    return () => {
      window.removeEventListener('mouseover', over);
      window.removeEventListener('mousedown', hide, true);
      window.removeEventListener('wheel', hide, true);
    };
  }, []);
  return tip ? <div className="w7-tooltip" style={{ left: tip.x, top: tip.y }}>{tip.text}</div> : null;
}
