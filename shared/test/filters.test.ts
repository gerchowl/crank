import { describe, expect, it } from "vitest";
import { createFilterChain } from "../src";

const run = (chain: ReturnType<typeof createFilterChain>, xs: (number | null)[], dt = 1) => xs.map((v, i) => chain.push(i * dt, v));

describe("filters", () => {
  it("empty chain is identity", () => {
    expect(run(createFilterChain(undefined), [1, null, 3])).toEqual([1, null, 3]);
  });

  it("sma averages over a time window", () => {
    const out = run(createFilterChain([{ kind: "sma", windowS: 3 }]), [3, 6, 9, 12, 15]);
    expect(out).toEqual([3, 4.5, 6, 9, 12]);
    // 4 Hz input: window counts seconds, not samples
    const fast = run(createFilterChain([{ kind: "sma", windowS: 1 }]), [0, 0, 0, 0, 4, 4, 4, 4], 0.25);
    expect(fast.at(-1)).toBe(4);
    expect(fast[4]).toBe(1);
  });

  it("ema has the right time constant regardless of sample rate", () => {
    for (const dt of [0.25, 1]) {
      const c = createFilterChain([{ kind: "ema", tauS: 10 }]);
      c.push(0, 0);
      let y: number | null = 0;
      for (let t = dt; t <= 10 + 1e-9; t += dt) y = c.push(t, 100);
      expect(y).toBeCloseTo(100 * (1 - Math.exp(-1)), 6);
    }
  });

  it("median removes spikes", () => {
    const out = run(createFilterChain([{ kind: "median", n: 3 }]), [100, 100, 900, 100, 100]);
    expect(out).toEqual([100, 100, 100, 100, 100]);
  });

  it("one_euro smooths jitter but follows big moves", () => {
    const c = createFilterChain([{ kind: "one_euro", minCutoff: 0.3, beta: 0.01 }]);
    let maxDev = 0;
    for (let i = 0; i < 200; i++) {
      const y = c.push(i * 0.25, 200 + (i % 2 ? 10 : -10)) as number;
      if (i > 40) maxDev = Math.max(maxDev, Math.abs(y - 200));
    }
    expect(maxDev).toBeLessThan(5);
    let y = 0;
    for (let i = 200; i < 240; i++) y = c.push(i * 0.25, 400) as number;
    expect(y).toBeGreaterThan(390);
  });

  it("hold fills nulls; other stages pass nulls through", () => {
    expect(run(createFilterChain([{ kind: "hold" }]), [null, 5, null, null, 7])).toEqual([null, 5, 5, 5, 7]);
    expect(run(createFilterChain([{ kind: "ema", tauS: 1 }]), [5, null, 5])).toEqual([5, null, 5]);
    const chain = run(createFilterChain([{ kind: "hold" }, { kind: "sma", windowS: 10 }]), [4, null, 8]);
    expect(chain).toEqual([4, 4, 16 / 3]);
  });

  it("resets when time goes backwards (new ride)", () => {
    const c = createFilterChain([{ kind: "sma", windowS: 100 }]);
    c.push(0, 100);
    c.push(1, 100);
    expect(c.push(0, 0)).toBe(0);
  });
});
