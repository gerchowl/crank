import type { Filter } from "./contract";

/**
 * Display filters. A chain is fed every *source* sample of a metric (in ride time), so
 * window lengths and time constants are in ride seconds regardless of display rate or
 * simulation speed. Null handling: `hold` replaces null with the last valid value; all
 * other stages pass null through unchanged and keep their state.
 */
export interface FilterStage {
  push(t: number, v: number | null): number | null;
  reset(): void;
}

class Sma implements FilterStage {
  private ts: number[] = [];
  private vs: number[] = [];
  private sum = 0;
  constructor(private windowS: number) {}
  push(t: number, v: number | null): number | null {
    if (v === null) return null;
    this.ts.push(t);
    this.vs.push(v);
    this.sum += v;
    while (this.ts.length > 1 && (this.ts[0] as number) <= t - this.windowS) {
      this.ts.shift();
      this.sum -= this.vs.shift() as number;
    }
    return this.sum / this.vs.length;
  }
  reset(): void {
    this.ts = [];
    this.vs = [];
    this.sum = 0;
  }
}

class Ema implements FilterStage {
  private y: number | null = null;
  private lastT = 0;
  constructor(private tauS: number) {}
  push(t: number, v: number | null): number | null {
    if (v === null) return null;
    if (this.y === null || this.tauS <= 0) {
      this.y = v;
    } else {
      const dt = Math.max(0, t - this.lastT);
      const a = 1 - Math.exp(-dt / this.tauS);
      this.y += a * (v - this.y);
    }
    this.lastT = t;
    return this.y;
  }
  reset(): void {
    this.y = null;
  }
}

class Median implements FilterStage {
  private buf: number[] = [];
  constructor(private n: number) {}
  push(_t: number, v: number | null): number | null {
    if (v === null) return null;
    this.buf.push(v);
    if (this.buf.length > this.n) this.buf.shift();
    const s = [...this.buf].sort((a, b) => a - b);
    const m = s.length >> 1;
    return s.length % 2 ? (s[m] as number) : ((s[m - 1] as number) + (s[m] as number)) / 2;
  }
  reset(): void {
    this.buf = [];
  }
}

/** 1€ filter (Casiez et al. 2012), time-based. */
class OneEuro implements FilterStage {
  private x: number | null = null;
  private dx = 0;
  private lastT = 0;
  constructor(private minCutoff: number, private beta: number, private dCutoff = 1) {}
  private static alpha(cutoff: number, dt: number): number {
    const tau = 1 / (2 * Math.PI * cutoff);
    return 1 / (1 + tau / dt);
  }
  push(t: number, v: number | null): number | null {
    if (v === null) return null;
    const dt = t - this.lastT;
    if (this.x === null || dt <= 0) {
      this.x = v;
      this.lastT = t;
      return v;
    }
    const rawDx = (v - this.x) / dt;
    this.dx += OneEuro.alpha(this.dCutoff, dt) * (rawDx - this.dx);
    const cutoff = this.minCutoff + this.beta * Math.abs(this.dx);
    this.x += OneEuro.alpha(cutoff, dt) * (v - this.x);
    this.lastT = t;
    return this.x;
  }
  reset(): void {
    this.x = null;
    this.dx = 0;
  }
}

class Hold implements FilterStage {
  private last: number | null = null;
  push(_t: number, v: number | null): number | null {
    if (v !== null) this.last = v;
    return this.last;
  }
  reset(): void {
    this.last = null;
  }
}

export function createFilter(f: Filter): FilterStage {
  switch (f.kind) {
    case "sma":
      return new Sma(Math.max(0, f.windowS));
    case "ema":
      return new Ema(Math.max(0, f.tauS));
    case "median":
      return new Median(Math.max(1, Math.round(f.n)));
    case "one_euro":
      return new OneEuro(Math.max(1e-3, f.minCutoff), Math.max(0, f.beta));
    case "hold":
      return new Hold();
  }
}

export interface FilterChain {
  push(t: number, v: number | null): number | null;
  reset(): void;
}

/** Compose filters left to right. Resets automatically if time goes backwards (new ride). */
export function createFilterChain(filters: readonly Filter[] | undefined): FilterChain {
  const stages = (filters ?? []).map(createFilter);
  let lastT = -Infinity;
  const reset = () => {
    for (const s of stages) s.reset();
    lastT = -Infinity;
  };
  return {
    push(t, v) {
      if (t < lastT) reset();
      lastT = t;
      let x = v;
      for (const s of stages) x = s.push(t, x);
      return x;
    },
    reset,
  };
}
