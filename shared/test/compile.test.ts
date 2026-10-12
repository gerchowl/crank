import { describe, expect, it } from "vitest";
import type { Workout } from "../src";
import { compileWorkout, demoWorkout, parseDsl, LAP_EXPECTED_S } from "../src";

const wk = (text: string, ftp = 280): Workout => {
  const r = parseDsl(text, ftp);
  if ("error" in r) throw new Error(r.error.message);
  return { id: "w", name: "w", ftp, nodes: r.nodes };
};

describe("compileWorkout", () => {
  it("flattens nested repeats with skipLastRest, paths and iterations", () => {
    const c = compileWorkout(wk("10m 50%; 2x[!last](3x(30s 120%, 15s 50%), 3m 50%); 10m 50%"));
    // 1 + 2*(3*2) + 1 rest (skipped on last) + 1 = 15
    expect(c.blocks).toHaveLength(1 + 12 + 1 + 1);
    const b = c.blocks[1]!;
    expect(b.path).toEqual([1, 0, 0]);
    expect(b.iteration).toEqual([0, 0]);
    const lastWork = c.blocks[13]!;
    expect(lastWork.path).toEqual([1, 0, 1]);
    expect(lastWork.iteration).toEqual([1, 2]);
    expect(c.blocks[14]!.path).toEqual([2]);
    expect(c.totalS).toBe(600 + 12 * 22.5 * 2 / 2 + 180 + 600);
    // contiguous start times
    c.blocks.forEach((x, i) => i && expect(x.startS).toBeCloseTo(c.blocks[i - 1]!.startS + c.blocks[i - 1]!.durS));
  });

  it("applies progression per rep to work steps only, cumulatively through nesting", () => {
    const c = compileWorkout(wk("3x[+5w](2x[+1%](1m 100%, 1m 50%))"));
    const work = c.blocks.filter((b) => b.phase === "work").map((b) => b.powerFrom);
    expect(work).toEqual([280, 282.8, 285, 287.8, 290, 292.8]);
    const rest = new Set(c.blocks.filter((b) => b.phase !== "work").map((b) => b.powerFrom));
    expect([...rest]).toEqual([140]);
  });

  it("classifies phases", () => {
    const c = compileWorkout(wk(`10m ramp 40-70%; 3x(3m 110%, 2m 50%); "Easy" 5m 55%; 2m free`));
    expect(c.blocks.map((b) => b.phase)).toEqual(["warmup", "work", "recovery", "work", "recovery", "work", "cooldown", "cooldown", "free"]);
  });

  it("computes NP/IF/TSS: one hour at FTP = 100 TSS", () => {
    const c = compileWorkout(wk("60m 100%"));
    expect(c.if).toBeCloseTo(1, 3);
    expect(c.tss).toBeCloseTo(100, 0);
    expect(c.timeInZoneS[3]).toBe(3600);
  });

  it("time in zone sums to total", () => {
    const c = compileWorkout(demoWorkout());
    expect(c.timeInZoneS.reduce((a, b) => a + b, 0)).toBeCloseTo(Math.round(c.totalS), -1);
  });

  it("gives duration ranges for hr_recovery and lap blocks", () => {
    const c = compileWorkout(wk("10m 50%; 5m 110%; hr<120 1m..4m 40%; lap 50%; 5m 50%"));
    const rec = c.blocks[2]!;
    expect(rec.durRangeS).toEqual([60, 240]);
    expect(rec.durS).toBeGreaterThanOrEqual(60);
    expect(rec.durS).toBeLessThanOrEqual(240);
    expect(rec.phase).toBe("recovery");
    expect(c.blocks[3]!.durS).toBe(LAP_EXPECTED_S);
    expect(c.blocks[3]!.durRangeS).toBeDefined();
    expect(c.totalRangeS[0]).toBeLessThan(c.totalS);
    expect(c.totalRangeS[1]).toBeGreaterThan(c.totalS);
  });

  it("estimates kJ blocks from power", () => {
    const c = compileWorkout(wk("100kj 250w"));
    expect(c.blocks[0]!.durS).toBeCloseTo(400);
  });

  it("validates max power, W'bal floor and missing cooldown", () => {
    const maxW = wk("5m 50%; 10s 300%; 5m 50%");
    maxW.nodes[1] = { type: "step", id: "sprint", end: { kind: "time", s: 10 }, target: { kind: "watts", value: 1500 } };
    const maxP = compileWorkout(maxW);
    expect(maxP.issues.find((i) => i.severity === "error")?.nodeId).toBe("sprint");
    expect(maxP.issues.some((i) => i.severity === "error" && /exceeds max/.test(i.message))).toBe(true);

    const wb = compileWorkout(wk("5m 50%; 10m 130%; 5m 50%"));
    expect(wb.issues.some((i) => i.severity === "error" && /W'bal/.test(i.message))).toBe(true);
    const tight = compileWorkout(wk("5m 50%; 6m 118%; 5m 50%"));
    expect(tight.issues.some((i) => i.severity === "warning" && /W'bal/.test(i.message))).toBe(true);

    const noCd = compileWorkout(wk("10m 50%; 20m 90%"));
    expect(noCd.issues.some((i) => /cool-down/.test(i.message))).toBe(true);
    const withCd = compileWorkout(wk("10m 50%; 20m 90%; 5m 50%"));
    expect(withCd.issues.some((i) => /cool-down/.test(i.message))).toBe(false);

    const unreachable = compileWorkout(wk("10m 50%; 5m 110%; hr<100 1m..3m 60%; 5m 50%"));
    expect(unreachable.issues.some((i) => /unreachable/.test(i.message))).toBe(true);
    expect(unreachable.blocks[2]!.durS).toBe(180);

    expect(compileWorkout({ id: "e", name: "e", ftp: 280, nodes: [] }).issues[0]!.message).toMatch(/empty/);
    expect(
      compileWorkout({ id: "e", name: "e", ftp: 280, nodes: [{ type: "repeat", id: "r", count: 0, children: [] }] }).issues.some((i) => i.nodeId === "r"),
    ).toBe(true);
  });

  it("the demo workout is ~45 min, valid, and has nested repeats + an hr_recovery block", () => {
    const w = demoWorkout();
    const c = compileWorkout(w);
    expect(c.totalS / 60).toBeGreaterThan(40);
    expect(c.totalS / 60).toBeLessThan(50);
    expect(c.issues.filter((i) => i.severity === "error")).toEqual([]);
    expect(c.blocks.some((b) => b.path.length === 3)).toBe(true);
    expect(c.blocks.some((b) => b.durRangeS)).toBe(true);
  });
});
