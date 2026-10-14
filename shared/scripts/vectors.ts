// ADR-0001 §3: execute the frozen mock, never duplicate its algorithms here.
import type { Filter, MetricId, Sample, Workout, WorkoutNode } from "../src/contract";
import {
  DEFAULT_ATHLETE, DEMO_DSL, SNIPPET_DSL, Rng, HistoryStore,
  parseDsl, printDsl, withoutIds, compileWorkout, normalizedPower,
  createFilterChain, wbalStep, wbalSeries, rmssd, dfaAlpha1,
  fitMonoExp, predictAt, buildHrRecoveryState, histogramFrom, heatmapFrom,
  mmpFrom, syntheticMmp, timeAgoEdges, timeAgoToU, uToTimeAgo,
  physioFor, hrSteady, powerForHr, powerZones, hrZones, zoneIndex,
  rewriteNodes,
} from "../src";
import { MetricBus } from "../src/bus";
import { RideSim, SIM_DT } from "../src/sim";
import { typicalDistribution } from "../src/distributions";

type Case = { name: string; input: unknown; output: unknown };
const vector = (name: string, input: unknown, output: unknown): Case => ({ name, input, output });
const samples = (values: (number | null)[], dt = 1): Sample[] => values.map((v, i) => ({ t: i * dt, v }));

function workout(text: string, ftp = 280): Workout {
  const parsed = parseDsl(text, ftp);
  if ("error" in parsed) throw new Error(`${text}: ${parsed.error.message}`);
  const identify = (nodes: WorkoutNode[], path: number[] = []): void => {
    nodes.forEach((n, i) => {
      const p = [...path, i];
      n.id = `n:${p.join(".")}`;
      if (n.type === "repeat") identify(n.children, p);
    });
  };
  identify(parsed.nodes);
  return { id: "w", name: "vector", ftp, nodes: parsed.nodes };
}

function dslVectors(): Case[] {
  const texts = [
    "", "  \n# nothing\n", DEMO_DSL, ...SNIPPET_DSL.map((s) => s.text),
    "10m ramp 50-75%; 3x[+5w, !last](4x(30s 120%, 15s 50%), 3m 50%); hr<120 2m..4m 40%; 10m 55%",
    '"A \\"q\\"" 1h5m30s 250w 85-95rpm; 10kj 52.5-60.5%; lap free; 20m 140bpm; hr<115 0s..5m 35% !coach; hr<115 1m..2m 35% !cap; 2x[-3%, +10w](1m 100% 90rpm, 30s 40%)',
    "# warm up\n10m 50%\n\n3x(1m 120%;1m 50%) # sets\n",
    "90s 100%; 3600s 50%; 3661s 50%",
    '"back\\\\slash 🚲" 4M RAMP 75-50% 90RPM; LAP FREE',
    "2x[+5w +2% !last](1m 100%, 30s 40%)",
  ];
  // Every shared/test/dsl.test.ts error, plus the remaining parser failure branches.
  const errors = [
    "10m 50%; 3x(1m 120%, 1m 50%", "10m 50%)", "10m 50%; 5q 120%", "10m",
    "10m 50% 60%", "10m 50% zzz", "3x[+5q](1m 50%)", "3x 1m 50%", "0x(1m 50%)",
    "hr<120 4m 40%", "hr<120 4m..2m 40%", "5m 50% !cap", "5m 900%", "5m 75-50%",
    '"unterminated 5m 50%', "10 50%", "5m 50% 2x(1m 50%)", '10m 50%\n"label"', "5m 50%; 3x()",
    "0s 50%", "5m ramp", "5m ramp 50%", "5m 50% 90rpm 95rpm", "5m 50% 95-90rpm",
    "3x[+5w", "3x[(](1m 50%)", "(", "1m 50% (", '"line\nbreak" 1m 50%',
    "hr<120", "5m 1500w", '"🚲" 5m 900%',
  ];
  return [...texts, ...errors].map((text, i) => {
    const parsed = parseDsl(text, 280);
    if (i >= texts.length && !("error" in parsed)) throw new Error(`Expected DSL error: ${text}`);
    return vector(`dsl-${i}`, { text, ftp: 280 }, "error" in parsed ? parsed : {
      nodes: withoutIds(parsed.nodes), printed: printDsl(parsed.nodes),
    });
  });
}

function compileVectors(): Case[] {
  const texts = [
    DEMO_DSL, "10m 50%; 2x[!last](3x(30s 120%, 15s 50%), 3m 50%); 10m 50%",
    "3x[+5w](2x[+1%](1m 100%, 1m 50%))",
    '10m ramp 40-70%; 3x(3m 110%, 2m 50%); "Easy" 5m 55%; 2m free',
    "60m 100%", "10m 50%; 5m 110%; hr<120 1m..4m 40%; lap 50%; 5m 50%",
    "100kj 250w", "5m 50%; 10m 130%; 5m 50%", "5m 50%; 6m 118%; 5m 50%",
    "10m 50%; 20m 90%", "10m 50%; 20m 90%; 5m 50%",
    "10m 50%; 5m 110%; hr<100 1m..3m 60%; 5m 50%", "", "10m 50%; 20m 65%; 5m 40%",
    "2m 140bpm; 1m 50-75%; 2x[-10w](1m 100%, 1m 50%)",
  ];
  const cases = texts.map((text, i) => {
    const w = workout(text);
    return vector(`compile-${i}`, { workout: w, options: {} }, compileWorkout(w));
  });
  const invalid: Workout[] = [
    { ...workout("1m 50%"), ftp: 0 },
    { id: "w", name: "vector", ftp: 280, nodes: [{ type: "repeat", id: "r", count: 0, children: [] }] },
    { id: "w", name: "vector", ftp: 280, nodes: [{ type: "repeat", id: "r", count: 1, children: [] }] },
    ...[0, -10].map((s): Workout => ({ ...workout("1m 50%"), nodes: [
      { type: "step", id: "s", end: { kind: "time", s }, target: { kind: "watts", value: 1500 } },
    ] })),
    { ...workout("1m 50%"), nodes: [{ type: "step", id: "s", end: { kind: "kj", kj: 0 }, target: { kind: "free" } }] },
    ...[20, 200].map((targetBpm): Workout => ({ ...workout("1m 50%"), nodes: [
      { type: "step", id: "s", end: { kind: "hr_recovery", minS: 120, maxS: 60, targetBpm, onUnreachable: "cap" }, target: { kind: "pct_ftp", value: 40 } },
    ] })),
  ];
  invalid.forEach((w, i) => cases.push(vector(`invalid-${i}`, { workout: w, options: {} }, compileWorkout(w))));
  const w = workout("5m 110%; hr<120 1m..4m 40%");
  const options = { athlete: { ftp: 300, weightKg: 80, restHr: 60, maxHr: 180 }, usualTauS: 80 };
  cases.push(vector("custom-athlete", { workout: w, options }, compileWorkout(w, options)));
  for (const power of [[], [100, 200], Array(30).fill(280), Array.from({ length: 65 }, (_, i) => i * 10)]) {
    cases.push(vector("normalizedPower", { power }, normalizedPower(power)));
  }
  return cases;
}

function filterVectors(): Case[] {
  const cases: Case[] = [];
  const run = (name: string, filters: Filter[], input: Sample[]) => {
    const chain = createFilterChain(filters);
    cases.push(vector(name, { filters, samples: input }, input.map(({ t, v }) => chain.push(t, v))));
  };
  run("identity", [], samples([1, null, 3]));
  run("sma", [{ kind: "sma", windowS: 3 }], samples([3, 6, 9, 12, 15]));
  run("sma-4hz", [{ kind: "sma", windowS: 1 }], samples([0, 0, 0, 0, 4, 4, 4, 4], 0.25));
  for (const dt of [0.25, 1]) run(`ema-${dt}`, [{ kind: "ema", tauS: 10 }], samples([0, ...Array(10 / dt).fill(100)], dt));
  run("median", [{ kind: "median", n: 3 }], samples([100, 100, 900, 100, 100]));
  run("one-euro", [{ kind: "one_euro", minCutoff: 0.3, beta: 0.01 }], samples(Array.from({ length: 240 }, (_, i) => i < 200 ? 200 + (i % 2 ? 10 : -10) : 400), 0.25));
  run("hold", [{ kind: "hold" }], samples([null, 5, null, null, 7]));
  run("ema-null", [{ kind: "ema", tauS: 1 }], samples([5, null, 5]));
  run("composed", [{ kind: "hold" }, { kind: "sma", windowS: 10 }], samples([4, null, 8]));
  for (const filters of [[{ kind: "sma", windowS: 100 }], [{ kind: "one_euro", minCutoff: 0, beta: -1 }], [{ kind: "median", n: 2 }], [{ kind: "ema", tauS: 0 }]] as Filter[][]) {
    run("backwards-and-duplicate-time", filters, [{ t: 0, v: 100 }, { t: 1, v: 100 }, { t: 0, v: 0 }, { t: 0, v: 10 }]);
  }
  return cases;
}

function historyVectors(): Case[] {
  const cases: Case[] = [];
  for (const [n, seed, hz] of [[1800, 1, 1], [7200, 2, 1], [10, 1, 4], [100, 1, 1]] as const) {
    const rng = new Rng(seed);
    const input = Array.from({ length: n * hz }, (_, i) => ({ t: i / hz, v: n === 10 ? (i % 4 === 0 ? 1000 : 100) : n === 100 ? (i >= 40 && i < 60 ? null : 1) : 200 + 100 * Math.sin(Math.floor(i / hz) / 50) + rng.normal(0, 20) }));
    const store = new HistoryStore();
    input.forEach(({ t, v }) => store.add("power", t, v));
    for (const scale of ["log", "fisheye"] as const) {
      const query = { metric: "power" as const, now: n - 0.25, spanS: n === 100 ? 3600 : n, buckets: n === 10 ? 10 : 100, scale };
      cases.push(vector(`history-${n}-${scale}`, { samples: input, query }, store.query(query.metric, query.now, query.spanS, query.buckets, scale)));
    }
    store.reset();
    cases.push(vector(`reset-${n}`, { samples: input, reset: true, query: { metric: "power", now: n, spanS: n, buckets: 10, scale: "log" } }, store.query("power", n, n, 10, "log")));
  }
  const store = new HistoryStore();
  store.add("power", 0, 100);
  cases.push(vector("metric-isolation", { samples: [{ t: 0, v: 100 }], query: { metric: "hr", now: 1, spanS: 10, buckets: 10, scale: "log" } }, store.query("hr", 1, 10, 10, "log")));
  for (const scale of ["log", "fisheye"] as const) {
    for (const spanS of [10, 1800, 7200]) cases.push(vector(`edges-${spanS}-${scale}`, { spanS, buckets: 100, scale }, {
      edges: timeAgoEdges(spanS, 100, scale),
      mapping: [-0.1, 0, 0.05, 0.5, 0.95, 1, 1.1].map((u) => ({ u, tAgo: uToTimeAgo(u, spanS, scale), back: timeAgoToU(uToTimeAgo(u, spanS, scale), spanS, scale) })),
    }));
  }
  return cases;
}

function physiologyVectors(): Record<string, Case[]> {
  const wbal = ([[5000, 280, 100], [100, 330, 10], [10000, 140, 20000 / 140], [20000, 350, 30], [17900, 140, 15]] as const)
    .map(([w, power, dt]) => vector("step", { w, power, dt, cp: 280, wPrime: 20000 }, wbalStep(w, power, dt, 280, 20000)));
  for (const [power, dt, n, wPrime] of [[380, 1, 60, 20000], [100, 0.25, 240, 30000], [100, 1, 60, 20000]] as const) {
    const powers = Array(n).fill(power);
    wbal.push(vector("series", { powers, dt, cp: 280, wPrime }, wbalSeries(powers, dt, 280, wPrime)));
  }
  const rng = new Rng(31);
  const hrv = [[], [800, 810], Array(120).fill(800), [800, 810, 790, 1200, 800, 805, 795, 800, 810], Array.from({ length: 120 }, () => 800 + rng.normal(0, 20)), Array.from({ length: 120 }, (_, i) => 800 + i + 10 * Math.sin(i))]
    .map((rr) => vector("hrv", { rr }, { rmssd: rmssd(rr), dfaAlpha1: dfaAlpha1(rr) }));
  const recovery: Case[] = [];
  const prior = { tauS: 60, tauLogSd: 0.35, hrInf: 110, hrInfSd: 8 };
  for (const tau of [20, 45, 90]) {
    const rng = new Rng(7);
    const ts = Array.from({ length: 3 * tau }, (_, i) => 1000 + i);
    const ys = ts.map((t) => Math.round(105 + 60 * Math.exp(-(t - 1000) / tau) + rng.normal(0, 1)));
    for (const n of [2, 5, 10, 30, ts.length]) {
      const input = { ts: ts.slice(0, n), ys: ys.slice(0, n), t0: 1000, prior };
      const fit = fitMonoExp(input.ts, input.ys, input.t0, prior);
      recovery.push(vector(`fit-${tau}-${n}`, input, { fit, prediction: fit ? predictAt(fit, 1180) : null }));
    }
  }
  for (const [hrInf, amp, tau, n, targetBpm] of [[105, 65, 45, 70, 120], [105, 65, 45, 30, 115], [95, 40, 15, 40, 120], [126, 40, 30, 120, 120], [105, 65, 45, 4, 120]] as const) {
    const rng = new Rng(3);
    const observed = Array.from({ length: n }, (_, i) => ({ t: 500 + i, v: i === 20 || i === 21 ? null : Math.round((i < 3 ? hrInf + amp - (3 - i) : hrInf + amp * Math.exp(-(i - 3) / tau)) + rng.normal(0, 0.8)) }));
    const input = { startedT: 500, minS: 60, maxS: 240, usualTauS: 50, priorHrInf: hrInf === 126 ? 124 : 108, targetBpm, observed, now: 500 + n - 1 };
    recovery.push(vector(`state-${hrInf}-${n}`, input, buildHrRecoveryState(input)));
  }
  const athlete = [DEFAULT_ATHLETE, { ftp: 220, weightKg: 80, restHr: 60, maxHr: 175 }].map((a) => vector("athlete", a, {
    physio: physioFor(a), powerZones: powerZones(a.ftp), hrZones: hrZones(a.maxHr),
    power: [0, 0.55, 0.75, 0.9, 1.05, 1.2, 1.5, 4].map((p) => ({ watts: p * a.ftp, hr: hrSteady(p * a.ftp, a), zone: zoneIndex(powerZones(a.ftp), p * a.ftp) })),
    hr: [50, 100, 140, 170].map((bpm) => ({ bpm, watts: powerForHr(bpm, a) })),
  }));
  return { wbal, hrv, recovery, athlete };
}

function distributionVectors(): Case[] {
  const cases: Case[] = [];
  const series = [[], [NaN, Infinity, -Infinity], [0, 10, 20, 20, 30, NaN], [-5, 0, 5, 10000], Array(60).fill(280)];
  for (const values of series) {
    for (const binWidth of [0, 10]) cases.push(vector("histogram", { metric: "power", series: values, binWidth, athlete: DEFAULT_ATHLETE }, histogramFrom("power", values, binWidth, typicalDistribution("power", DEFAULT_ATHLETE))));
    cases.push(vector("mmp", { power: values, durations: [1, 2, 3, 5, 30, 60, 120] }, mmpFrom(values, [1, 2, 3, 5, 30, 60, 120])));
    const ys = values.map((v) => v / 2).slice(0, -1);
    cases.push(vector("heatmap", { x: "power", y: "hr", xs: values, ys, xBin: 10, yBin: 5 }, heatmapFrom("power", "hr", values, ys, 10, 5)));
  }
  cases.push(vector("syntheticMmp", { athlete: DEFAULT_ATHLETE, scale: 0.9 }, syntheticMmp(DEFAULT_ATHLETE, 0.9)));
  return cases;
}

function simulatorVectors(): Case[] {
  const w = workout("30s 50%; 30s 120%; hr<120 5s..30s 40%; 30s free");
  // Harder ride (#67 review): 190 % FTP for 230 s spends > 2.5 W′ (its 90 s recovery then runs on the capped fatigue term) and drives the
  // intensity EMA past 1.15, reaching the RR generator's anti-correlated and
  // saturated branches that the 120 s vectors above never enter.
  const hard = workout("20s 60%; 230s 190%; 90s 40%; 40s 150%");
  const runs: { w: Workout; seed: number; ticks: number }[] = [
    { w, seed: 1, ticks: 480 },
    { w, seed: 31, ticks: 480 },
    { w: hard, seed: 7, ticks: 1520 },
  ];
  return runs.map(({ w, seed, ticks }) => {
    const bus = new MetricBus();
    const events: unknown[] = [];
    const sim = new RideSim(DEFAULT_ATHLETE, seed, bus, {
      statusChanged: () => events.push({ status: sim.status() }),
      evaluate: (t) => events.push({ evaluate: t }),
      recoveryEnded: (state, step) => events.push({ recoveryEnded: state, nodeId: step.id }),
      recoveryUnreachable: (state, step) => events.push({ recoveryUnreachable: state, nodeId: step.id }),
    });
    const metrics: MetricId[] = ["power", "hr", "cadence", "rr", "wbal_pct", "speed"];
    const published: unknown[] = [];
    metrics.forEach((metric) => bus.listen(metric, (t, v, reset) => published.push({ metric, t, v, reset })));
    sim.load(w);
    sim.start();
    const snapshots: unknown[] = [];
    for (let tick = 0; tick < ticks; tick++) {
      sim.step();
      if (tick % 40 === 39) snapshots.push({ status: sim.status(), features: sim.features(), recovery: sim.recoveryState() });
    }
    return vector(`sim-${seed}`, { workout: w, athlete: DEFAULT_ATHLETE, seed, ticks, dt: SIM_DT, metrics }, { published, snapshots, events });
  });
}

export function exportVectors(): Record<string, Case[]> {
  const w = workout(DEMO_DSL);
  return {
    dsl: dslVectors(), compile: compileVectors(), filters: filterVectors(), history: historyVectors(),
    ...physiologyVectors(), distributions: distributionVectors(), simulator: simulatorVectors(),
    rewrite: (["harder", "easier", "shorter_same_stimulus", "vary"] as const).map((intent) => vector(intent, { nodes: withoutIds(w.nodes), intent, ftp: w.ftp, seed: 31 }, withoutIds(rewriteNodes(w.nodes, intent, w.ftp, new Rng(31))))),
  };
}

export function serializeVectors(cases: Case[]): string {
  return JSON.stringify({ schemaVersion: 1, cases }, (_key, value: unknown) =>
    typeof value === "number" && !Number.isFinite(value) ? String(value) : value, 2) + "\n";
}
