/**
 * UI-side logic analyzer buffer. Receives incremental pin-change chunks from the worker and
 * keeps the most recent N transitions in typed arrays (outside React for zero-copy drawing).
 */
const CAPACITY = 1 << 20;

class TraceBuffer {
  readonly cycles = new Float64Array(CAPACITY);
  /** 32-bit words per entry (`ceil(GPIOs / 32)`), flattened in `levels`. */
  private words = 1;
  private levels = new Uint32Array(CAPACITY);
  /** Number of valid entries (<= CAPACITY), oldest first starting at `start`. */
  count = 0;
  private start = 0;
  /** Incremented on every change; consumers compare to know when to redraw. */
  version = 0;
  /** Cycle of the most recent state (end of the visible signal). */
  endCycle = 0;
  hz = 1_000_000;
  private listeners = new Set<() => void>();

  clear(): void {
    this.count = 0;
    this.start = 0;
    this.endCycle = 0;
    this.bump();
  }

  append(cycles: Float64Array, levels: Uint32Array, endCycle: number, hz: number, words = 1): void {
    this.hz = hz;
    if (words !== this.words) {
      // Another device with a different pin count: the stored history has the old layout.
      this.words = words;
      this.levels = new Uint32Array(CAPACITY * words);
      this.count = 0;
      this.start = 0;
    }
    for (let i = 0; i < cycles.length; i++) {
      // Worker restarted the trace (power cycle / load): drop stale history.
      if (this.count > 0 && cycles[i] < this.cycleAt(this.count - 1)) this.clear();
      const idx = (this.start + this.count) % CAPACITY;
      this.cycles[idx] = cycles[i];
      for (let w = 0; w < words; w++) this.levels[idx * words + w] = levels[i * words + w];
      if (this.count < CAPACITY) this.count++;
      else this.start = (this.start + 1) % CAPACITY;
    }
    this.endCycle = Math.max(endCycle, this.count ? this.cycleAt(this.count - 1) : 0);
    this.bump();
  }

  cycleAt(i: number): number {
    return this.cycles[(this.start + i) % CAPACITY];
  }

  /** Level (0/1) of pin `pin` in entry `i`. */
  bitAt(i: number, pin: number): number {
    return (this.levels[((this.start + i) % CAPACITY) * this.words + (pin >> 5)] >>> (pin & 31)) & 1;
  }

  /** Index of the last entry with cycle <= c (or -1). Binary search. */
  indexAt(c: number): number {
    let lo = 0;
    let hi = this.count - 1;
    let ans = -1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (this.cycleAt(mid) <= c) {
        ans = mid;
        lo = mid + 1;
      } else hi = mid - 1;
    }
    return ans;
  }

  /** The newest `max` entries (pop-out window snapshots). */
  export(max: number): { cycles: number[]; levels: number[]; words: number; endCycle: number; hz: number } {
    const n = Math.min(max, this.count);
    const w = this.words;
    const cycles: number[] = new Array(n);
    const levels: number[] = new Array(n * w);
    for (let i = 0; i < n; i++) {
      const e = (this.start + this.count - n + i) % CAPACITY;
      cycles[i] = this.cycles[e];
      for (let k = 0; k < w; k++) levels[i * w + k] = this.levels[e * w + k];
    }
    return { cycles, levels, words: w, endCycle: this.endCycle, hz: this.hz };
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private bump(): void {
    this.version++;
    for (const f of this.listeners) f();
  }
}

export const trace = new TraceBuffer();
