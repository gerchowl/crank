import { describe, expect, it } from "vitest";
import { wbalSeries, wbalStep } from "../src";

const CP = 280;
const W0 = 20000;

describe("W'bal (Skiba differential)", () => {
  it("depletes linearly above CP", () => {
    const w = wbalSeries(new Array(60).fill(380), 1, CP, W0);
    expect(w.at(-1)).toBeCloseTo(W0 - 100 * 60);
  });

  it("refills exponentially below CP with tau = W0 / (CP - P)", () => {
    const tau = W0 / (CP - 140);
    let w = 10000;
    w = wbalStep(w, 140, tau, CP, W0);
    expect(w).toBeCloseTo(W0 - 10000 * Math.exp(-1), 6);
  });

  it("is step-size independent for constant power", () => {
    const a = wbalSeries(new Array(240).fill(100), 0.25, CP, 10000 + W0);
    let w = 5000;
    for (let i = 0; i < 240; i++) w = wbalStep(w, 100, 0.25, CP, W0);
    let w2 = 5000;
    for (let i = 0; i < 60; i++) w2 = wbalStep(w2, 100, 1, CP, W0);
    expect(w).toBeCloseTo(w2, 9);
    expect(a.length).toBe(240);
  });

  it("does not recover at exactly CP and can go negative", () => {
    expect(wbalStep(5000, CP, 100, CP, W0)).toBeCloseTo(5000);
    expect(wbalStep(100, CP + 50, 10, CP, W0)).toBe(-400);
  });

  it("matches a 30/15 hand calculation", () => {
    // 30 s @ 350 W: −2100 J; 15 s @ 140 W: refill (W0−w)(1−e^(−140·15/W0))
    let w = W0;
    w = wbalStep(w, 350, 30, CP, W0);
    expect(w).toBe(W0 - 2100);
    w = wbalStep(w, 140, 15, CP, W0);
    expect(w).toBeCloseTo(W0 - 2100 * Math.exp((-140 * 15) / W0), 9);
  });
});
