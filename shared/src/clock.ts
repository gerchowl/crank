/**
 * Wall-clock abstraction. The backend only uses timers through this interface so that
 * tests (and headless tools) can drive it with a VirtualClock deterministically.
 */
export interface Clock {
  now(): number; // ms
  setInterval(fn: () => void, ms: number): number;
  clearInterval(id: number): void;
  setTimeout(fn: () => void, ms: number): number;
  clearTimeout(id: number): void;
}

export const realClock: Clock = {
  now: () => (typeof performance !== "undefined" ? performance.now() : Date.now()),
  setInterval: (fn, ms) => globalThis.setInterval(fn, ms) as unknown as number,
  clearInterval: (id) => globalThis.clearInterval(id as unknown as ReturnType<typeof setInterval>),
  setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms) as unknown as number,
  clearTimeout: (id) => globalThis.clearTimeout(id as unknown as ReturnType<typeof setTimeout>),
};

interface VTimer { id: number; due: number; every: number | null; fn: () => void }

/** Manually advanced clock. Timers fire in due-time order during advance(). */
export class VirtualClock implements Clock {
  private t = 0;
  private seq = 0;
  private timers = new Map<number, VTimer>();

  now(): number {
    return this.t;
  }
  setInterval(fn: () => void, ms: number): number {
    const id = ++this.seq;
    const every = Math.max(1, ms);
    this.timers.set(id, { id, due: this.t + every, every, fn });
    return id;
  }
  clearInterval(id: number): void {
    this.timers.delete(id);
  }
  setTimeout(fn: () => void, ms: number): number {
    const id = ++this.seq;
    this.timers.set(id, { id, due: this.t + Math.max(0, ms), every: null, fn });
    return id;
  }
  clearTimeout(id: number): void {
    this.timers.delete(id);
  }
  get pendingTimers(): number {
    return this.timers.size;
  }
  advance(ms: number): void {
    const end = this.t + Math.max(0, ms);
    for (;;) {
      let next: VTimer | undefined;
      for (const tm of this.timers.values()) if (tm.due <= end && (!next || tm.due < next.due || (tm.due === next.due && tm.id < next.id))) next = tm;
      if (!next) break;
      this.t = next.due;
      if (next.every === null) this.timers.delete(next.id);
      else next.due += next.every;
      next.fn();
    }
    this.t = end;
  }
}
