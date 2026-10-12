import { describe, expect, it } from "vitest";
import type { CoachNudge, MetricId, RideStatus, Sample } from "../src";
import { createMockBackend, LAYOUT_STORAGE_KEY, memoryStorage, VirtualClock, withoutIds } from "../src";

const mk = (o: Parameters<typeof createMockBackend>[0] = {}) => createMockBackend({ clock: "virtual", storage: null, ...o });
const flush = () => new Promise<void>((r) => queueMicrotask(r));

describe("createMockBackend", () => {
  it("loads the demo and exposes catalog, zones and athlete", () => {
    const b = mk();
    expect(b.athlete).toEqual({ ftp: 280, weightKg: 72, maxHr: 190, restHr: 50 });
    expect(b.metrics).toHaveLength(33);
    expect(b.zones.power).toHaveLength(7);
    expect(b.zones.hr).toHaveLength(5);
    expect(b.ride.current()!.compiled.blocks.length).toBeGreaterThan(10);
    b.dispose();
  });

  it("publishes every metric during a ride", async () => {
    const b = mk({ speed: 60 });
    const seen = new Map<MetricId, number>();
    for (const m of b.metrics) b.subscribe(m.id, { rateHz: 10 }, (s) => s.v !== null && seen.set(m.id, s.v));
    b.ride.start();
    b.advance(1200);
    await flush();
    const missing = b.metrics.map((m) => m.id).filter((id) => !seen.has(id));
    expect(missing).toEqual([]);
    expect(seen.get("w_per_kg")).toBeLessThan(10);
    b.dispose();
  });

  it("is deterministic for a seed and differs across seeds", () => {
    const run = (seed: number) => {
      const b = mk({ seed, speed: 60 });
      b.ride.start();
      b.advance(900);
      const h = b.history({ metric: "hr", spanS: 900, buckets: 40, scale: "log" });
      const p = b.history({ metric: "power", spanS: 900, buckets: 40, scale: "fisheye" });
      b.dispose();
      return JSON.stringify([h, p]);
    };
    expect(run(5)).toBe(run(5));
    expect(run(5)).not.toBe(run(6));
  });

  it("the sim does not depend on wall-clock granularity", () => {
    const a = mk({ speed: 10 });
    const b = mk({ speed: 60 });
    a.ride.start();
    b.ride.start();
    a.advance(600);
    for (let i = 0; i < 60; i++) b.advance(10);
    const q = { metric: "power_30s" as const, spanS: 600, buckets: 50, scale: "log" as const };
    expect(a.history(q)).toEqual(b.history(q));
  });

  it("produces a physiologically plausible ride", () => {
    const b = mk({ speed: 60 });
    b.ride.start();
    b.advance(3600);
    expect(b.onStatus).toBeDefined();
    let st: RideStatus | undefined;
    b.onStatus((s) => (st = s))();
    expect(st!.state).toBe("finished");
    const hr = b.histogram("hr", 5);
    const lo = hr.bins.find((x) => x.seconds > 0)!.lo;
    const hi = hr.bins.filter((x) => x.seconds > 0).at(-1)!.lo;
    expect(lo).toBeGreaterThanOrEqual(55);
    expect(hi).toBeLessThanOrEqual(190);
    expect(hi).toBeGreaterThan(165);
    const cad = b.histogram("cadence", 5);
    const modal = cad.bins.reduce((m, x) => (x.seconds > m.seconds ? x : m));
    expect(modal.lo).toBeGreaterThanOrEqual(80);
    expect(modal.lo).toBeLessThanOrEqual(95);
    // three hr_recovery blocks, each with a fit, τ in a plausible band and slowing with fatigue
    const recs = b.pastRecoveries();
    expect(recs).toHaveLength(3);
    for (const r of recs) {
      expect(r.fit!.tauS).toBeGreaterThan(30);
      expect(r.fit!.tauS).toBeLessThan(100);
      expect(r.observed.length).toBeGreaterThanOrEqual(120);
      expect(r.observed.length).toBeLessThanOrEqual(241);
    }
    expect(recs[2]!.fit!.tauS).toBeGreaterThan(recs[0]!.fit!.tauS);
    // there is one trainer blip (null power) and some HR dropouts
    const power = b.history({ metric: "power", spanS: 7200, buckets: 2000, scale: "log" });
    expect(power.length).toBeGreaterThan(0);
    const mmp = b.mmp();
    expect(mmp.durationsS[0]).toBe(1);
    expect(mmp.durationsS.at(-1)).toBe(3600);
    expect(mmp.today[0]).toBeGreaterThan(mmp.today[10]!);
    for (let i = 1; i < mmp.allTime.length; i++) expect(mmp.allTime[i]!).toBeLessThanOrEqual(mmp.allTime[i - 1]!);
    for (let i = 0; i < mmp.allTime.length; i++) expect(mmp.allTime[i]!).toBeGreaterThan(mmp.best90d[i]!);
    expect(mmp.allTime.at(-1)).toBeGreaterThan(250);
    expect(mmp.allTime.at(-1)).toBeLessThan(310);
    const tss = b.history({ metric: "tss", spanS: 5, buckets: 1, scale: "log" }).at(-1)!.mean;
    expect(tss).toBeGreaterThan(35);
    expect(tss).toBeLessThan(80);
    b.dispose();
  });

  it("has a trainer 'connecting' blip and HR sensor dropouts", () => {
    const b = mk({ speed: 60 });
    const trainer = new Set<string>();
    const hrs = new Set<string>();
    b.onStatus((s) => {
      trainer.add(s.trainer);
      hrs.add(s.hrSensor);
    });
    let nulls = 0;
    b.subscribe("hr", { rateHz: 10 }, (s) => s.v === null && nulls++);
    b.ride.start();
    for (let i = 0; i < 2700; i++) b.advance(1);
    expect([...trainer].sort()).toEqual(["connected", "connecting"]);
    expect(hrs.has("lost")).toBe(true);
    expect(nulls).toBeGreaterThan(0);
    b.dispose();
  });
});

describe("subscribe", () => {
  it("emits at the requested display rate, only when new data arrived", async () => {
    const b = mk({ speed: 1 });
    const got: Sample[] = [];
    b.subscribe("power", { rateHz: 2 }, (s) => got.push(s));
    await flush();
    expect(got).toEqual([{ t: 0, v: null }]); // initial sample (async)
    b.ride.start();
    b.advanceReal(10_000);
    // 2 Hz for 10 s → 20 emissions (+ initial)
    expect(got.length).toBeGreaterThanOrEqual(20);
    expect(got.length).toBeLessThanOrEqual(21);
    b.ride.pause();
    b.advanceReal(1000); // flushes the last unsent sample
    const n = got.length;
    b.advanceReal(5000);
    expect(got.length).toBe(n); // nothing new while paused
    b.dispose();
  });

  it("caps at the source rate and clamps rateHz to 0.1–10", () => {
    const b = mk({ speed: 1 });
    const fast: Sample[] = [];
    const slow: Sample[] = [];
    b.subscribe("hr", { rateHz: 100 }, (s) => fast.push(s));
    b.subscribe("hr", { rateHz: 0.01 }, (s) => slow.push(s));
    b.ride.start();
    b.advanceReal(30_000);
    expect(fast.length).toBeGreaterThan(25); // HR is 1 Hz at 1×
    expect(fast.length).toBeLessThan(35);
    expect(slow.length).toBe(3); // 0.1 Hz
    b.dispose();
  });

  it("applies filters per subscription", () => {
    const b = mk({ speed: 10 });
    const raw: number[] = [];
    const smooth: number[] = [];
    b.subscribe("power", { rateHz: 10 }, (s) => s.v !== null && raw.push(s.v));
    b.subscribe("power", { rateHz: 10, filter: [{ kind: "median", n: 5 }, { kind: "ema", tauS: 5 }] }, (s) => s.v !== null && smooth.push(s.v));
    b.ride.start();
    b.advance(400);
    const jitter = (xs: number[]) => xs.slice(1).reduce((a, x, i) => a + Math.abs(x - xs[i]!), 0) / xs.length;
    expect(jitter(smooth)).toBeLessThan(jitter(raw) / 3);
    b.dispose();
  });

  it("unsubscribe stops callbacks and clears timers; dispose clears everything", async () => {
    const clock = new VirtualClock();
    const b = createMockBackend({ clock, storage: null, speed: 10 });
    const base = clock.pendingTimers;
    let calls = 0;
    const unsubs = [0, 1, 2].map(() => b.subscribe("hr", { rateHz: 5 }, () => calls++));
    expect(clock.pendingTimers).toBe(base + 3);
    unsubs.forEach((u) => u());
    unsubs[0]!(); // idempotent
    expect(clock.pendingTimers).toBe(base);
    await flush();
    expect(calls).toBe(0); // even the initial async emission is cancelled
    b.ride.start();
    b.subscribe("power", { rateHz: 5 }, () => calls++);
    void b.planner.propose({ sleep: 3, legs: 3, stress: 3, motivation: 3, minutes: 45 });
    expect(clock.pendingTimers).toBeGreaterThan(base);
    b.dispose();
    expect(clock.pendingTimers).toBe(0);
  });

  it("resets filter state and emits null when a new workout is loaded", async () => {
    const b = mk({ speed: 60 });
    const got: Sample[] = [];
    b.subscribe("power_30s", { rateHz: 10, filter: [{ kind: "hold" }] }, (s) => got.push(s));
    b.ride.start();
    b.advance(120);
    b.ride.load(b.ride.current()!.workout);
    b.advanceReal(200);
    expect(got.at(-1)).toEqual({ t: 0, v: null });
    b.dispose();
  });
});

describe("ride control", () => {
  it("start / pause / resume / stop / skip / adjust / speed", () => {
    const b = mk({ speed: 10 });
    const st: RideStatus[] = [];
    b.onStatus((s) => st.push(s));
    expect(st[0]!.state).toBe("idle");
    b.ride.start();
    b.advance(30);
    b.ride.pause();
    const tPaused = st.at(-1)!.t;
    b.advanceReal(5000);
    expect(st.at(-1)!.state).toBe("paused");
    expect(st.at(-1)!.t).toBe(tPaused);
    b.ride.resume();
    b.ride.skipBlock();
    expect(st.at(-1)!.blockIndex).toBe(1);
    b.ride.adjust(5);
    b.ride.adjust(5);
    expect(st.at(-1)!.manualOffsetPct).toBe(10);
    b.ride.setSpeed(60);
    expect(st.at(-1)!.speed).toBe(60);
    b.ride.stop();
    expect(st.at(-1)!.state).toBe("finished");
    b.dispose();
  });

  it("ERG power follows the (manually offset) target", () => {
    const b = mk({ speed: 10 });
    b.ride.start();
    b.advance(200);
    b.ride.adjust(10);
    b.advance(60);
    const tgt = b.history({ metric: "target_power", spanS: 20, buckets: 1, scale: "log" }).at(-1)!.mean;
    const p = b.history({ metric: "power", spanS: 20, buckets: 1, scale: "log" }).at(-1)!.mean;
    expect(Math.abs(p - tgt) / tgt).toBeLessThan(0.06);
    const block = b.ride.current()!.compiled.blocks[0]!;
    const expected = (block.powerFrom + ((block.powerTo - block.powerFrom) * 255) / block.durS) * 1.1;
    expect(tgt).toBeGreaterThan(expected * 0.97);
    expect(tgt).toBeLessThan(expected * 1.03);
    b.dispose();
  });

  it("hrRecovery() is live during hr_recovery blocks", () => {
    const b = mk({ speed: 60 });
    b.ride.start();
    let active = 0;
    let withFit = 0;
    for (let i = 0; i < 1500; i += 5) {
      b.advance(5);
      const r = b.hrRecovery();
      if (r.active) {
        active++;
        if (r.fit) {
          withFit++;
          expect(r.targetBpm).toBe(118);
          expect(r.readyInS!.lo).toBeLessThanOrEqual(r.readyInS!.mean + 1e-9);
          expect(r.readyInS!.hi).toBeGreaterThanOrEqual(r.readyInS!.mean - 1e-9);
          expect(r.predicted!.length).toBeGreaterThan(0);
        }
      }
    }
    expect(active).toBeGreaterThan(20);
    expect(withFit).toBeGreaterThan(active * 0.8);
    b.dispose();
  });
});

describe("coach", () => {
  it("emits nudges with consistent bands, auto-applies strong ones and expires lean ones", () => {
    const b = mk({ speed: 60 });
    const events: CoachNudge[] = [];
    b.onNudge((n) => events.push(n));
    b.ride.start();
    b.advance(3000);
    const d = b.coach.decisions();
    expect(d.length).toBeGreaterThan(0);
    for (const n of d) {
      const band = n.p >= 0.85 ? "strong" : n.p >= 0.65 ? "lean" : "uncertain";
      expect(n.band).toBe(band);
      expect(n.status).toBe(band === "strong" ? "auto_applied" : "expired");
    }
    expect(events.length).toBeGreaterThanOrEqual(d.length);
    if (d.some((n) => n.status === "auto_applied" && n.proposedPctChange)) {
      expect(b.ride.current()!.compiled.blocks.some((x) => x.aiChanged)).toBe(true);
    }
    b.dispose();
  });

  it("respond() accepts a pending nudge and applies it", () => {
    const b = mk({ speed: 60, seed: 42 });
    let pending: CoachNudge | undefined;
    b.onNudge((n) => {
      if (n.status === "pending" && n.proposedPctChange && !pending) pending = n;
    });
    b.ride.start();
    for (let i = 0; i < 3000 && !pending; i += 5) b.advance(5);
    expect(pending).toBeDefined();
    b.coach.respond(pending!.callId, true);
    const after = b.coach.decisions().find((n) => n.callId === pending!.callId)!;
    expect(after.status).toBe("accepted");
    expect(b.ride.current()!.compiled.blocks.some((x) => x.aiChanged)).toBe(true);
    b.coach.rate(after.callId, "right", true);
    b.coach.respond(after.callId, false); // no-op once decided
    expect(b.coach.decisions().find((n) => n.callId === pending!.callId)!.status).toBe("accepted");
    b.dispose();
  });
});

describe("planner", () => {
  const checkin = { sleep: 5, legs: 4, stress: 2, motivation: 5, minutes: 60 } as const;

  it("propose resolves after ~800 ms with a valid workout that fits the time", async () => {
    const b = mk();
    let done = false;
    const p = b.planner.propose(checkin).then((x) => {
      done = true;
      return x;
    });
    b.advanceReal(550);
    await flush();
    expect(done).toBe(false);
    b.advanceReal(500);
    const prop = await p;
    const c = b.compileWorkout(prop.workout);
    expect(c.issues.filter((i) => i.severity === "error")).toEqual([]);
    expect(Math.abs(c.totalS / 60 - 60)).toBeLessThan(12);
    expect(prop.rationale.length).toBeGreaterThan(20);
    b.dispose();
  });

  it("propose varies with the check-in", async () => {
    const b = mk();
    const ps = [
      b.planner.propose(checkin),
      b.planner.propose({ sleep: 2, legs: 2, stress: 4, motivation: 2, minutes: 45 }),
      b.planner.propose({ sleep: 1, legs: 1, stress: 5, motivation: 1, minutes: 30, note: "feeling sick" }),
    ];
    b.advanceReal(1100);
    const names = (await Promise.all(ps)).map((p) => p.workout.name);
    expect(new Set(names).size).toBe(3);
    b.dispose();
  });

  it("rewrite produces sensible transforms", async () => {
    const b = mk();
    const r = b.parseDsl("10m 50%; 4x[!last](4m 110%, 3m 50%); 5m 50%", 280);
    if ("error" in r) throw new Error();
    const rw = async (intent: Parameters<typeof b.planner.rewrite>[1]) => {
      const p = b.planner.rewrite(r.nodes, intent);
      b.advanceReal(1000);
      return p;
    };
    const base = b.compileWorkout({ id: "a", name: "a", ftp: 280, nodes: r.nodes });
    const comp = (nodes: typeof r.nodes) => b.compileWorkout({ id: "a", name: "a", ftp: 280, nodes });

    const harder = comp(await rw("harder"));
    expect(harder.tss).toBeGreaterThan(base.tss);
    const easier = comp(await rw("easier"));
    expect(easier.tss).toBeLessThan(base.tss);
    const shorter = comp(await rw("shorter_same_stimulus"));
    expect(shorter.totalS).toBeLessThan(base.totalS * 0.85);
    expect(shorter.if).toBeGreaterThan(base.if);
    const v1 = await rw("vary");
    const v2 = await rw("vary");
    expect(withoutIds(v1)).not.toEqual(withoutIds(r.nodes));
    void v2;
    const varied = comp(v1);
    expect(Math.abs(varied.tss - base.tss) / base.tss).toBeLessThan(0.25);
    // inputs untouched, modified nodes keep their ids
    expect(r.nodes[1]!.id).toBe((await rw("harder"))[1]!.id);
    b.dispose();
  });
});

describe("layouts", () => {
  it("ships laptop/tablet/tv with Interval/Recovery/Nerd pages using every widget kind", () => {
    const b = mk();
    const ls = b.layouts.list();
    expect(ls.map((l) => l.device)).toEqual(["laptop", "tablet", "tv"]);
    const kinds = ["big_number", "number_sparkline", "zone_bar", "log_time_chart", "histogram", "heatmap2d", "mmp_curve", "workout_strip", "hr_recovery", "coach_card"];
    for (const l of ls) {
      expect(l.pages.map((p) => p.name)).toEqual(["Interval", "Recovery", "Nerd"]);
      const used = new Set(l.pages.flatMap((p) => p.widgets.map((w) => w.kind)));
      if (l.device === "laptop") expect([...used].sort()).toEqual([...kinds].sort());
      for (const p of l.pages)
        for (const w of p.widgets) {
          expect(w.x + w.w).toBeLessThanOrEqual(12);
          expect(w.rateHz).toBeGreaterThanOrEqual(0.1);
          expect(w.rateHz).toBeLessThanOrEqual(10);
        }
    }
    b.dispose();
  });

  it("persists saves and resets to the built-in default", () => {
    const storage = memoryStorage();
    const a = createMockBackend({ clock: "virtual", storage });
    const l = a.layouts.get("laptop")!;
    l.pages[0]!.widgets.pop();
    l.name = "Mine";
    a.layouts.save(l);
    a.layouts.save({ ...l, id: "custom", name: "Custom" });
    expect(storage.getItem(LAYOUT_STORAGE_KEY)).toContain("Mine");
    const b = createMockBackend({ clock: "virtual", storage });
    expect(b.layouts.get("laptop")!.name).toBe("Mine");
    expect(b.layouts.list().map((x) => x.id)).toEqual(["laptop", "tablet", "tv", "custom"]);
    const reset = b.layouts.reset("laptop");
    expect(reset.name).toBe("Laptop");
    expect(b.layouts.get("laptop")!.pages[0]!.widgets.length).toBe(reset.pages[0]!.widgets.length);
    // returned objects are copies
    b.layouts.get("tv")!.name = "mutated";
    expect(b.layouts.get("tv")!.name).toBe("TV");
    a.dispose();
    b.dispose();
  });
});

describe("realtime clock", () => {
  it("advances with real timers at the selected speed and cleans up", async () => {
    const b = createMockBackend({ storage: null, speed: 60 });
    const got: Sample[] = [];
    b.subscribe("elapsed", { rateHz: 10 }, (s) => got.push(s));
    b.ride.start();
    await new Promise((r) => setTimeout(r, 600));
    b.dispose();
    const t = got.at(-1)!.v!;
    expect(t).toBeGreaterThan(15); // ~36 s of ride time in 0.6 s at 60×
    expect(t).toBeLessThan(60);
    const n = got.length;
    await new Promise((r) => setTimeout(r, 200));
    expect(got.length).toBe(n);
  });
});
