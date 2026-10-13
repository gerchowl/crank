import type {
  Backend,
  Bucket,
  Checkin,
  CoachNudge,
  Heatmap2D,
  RideStreams,
  Histogram,
  HistoryQuery,
  HrRecoveryState,
  MetricId,
  MmpCurve,
  Proposal,
  RewriteIntent,
  RideStatus,
  Sample,
  SubscribeOpts,
  Unsubscribe,
  Workout,
  WorkoutNode,
} from "./contract";
import { type Athlete, DEFAULT_ATHLETE, hrZones, metricCatalog, powerZones } from "./athlete";
import { MetricBus } from "./bus";
import { type Clock, realClock, VirtualClock } from "./clock";
import { Coach } from "./coach";
import { heatmapFrom, histogramFrom, MMP_DURATIONS_S, mmpFrom, syntheticMmp, typicalDistribution } from "./distributions";
import { createFilterChain } from "./filters";
import { browserStorage, LayoutStore, memoryStorage, type StorageLike } from "./layouts";
import { plannerLatencyMs, proposeWorkout } from "./planner";
import { hashString, Rng } from "./rng";
import { RideSim, SIM_DT } from "./sim";
import { clamp } from "./util";
import { compileWorkout } from "./workout/compile";
import { parseDsl, printDsl } from "./workout/dsl";
import { demoWorkout, seedSnippets } from "./workout/library";
import { rewriteNodes } from "./workout/rewrite";

export interface MockBackendOptions {
  /** RNG seed: same seed ⇒ same ride. Default 42. */
  seed?: number;
  athlete?: Partial<Athlete>;
  /** Workout loaded at creation. Default: the 45-min demo. `null` = nothing loaded. */
  workout?: Workout | null;
  /** Start riding immediately. Default false. */
  autoStart?: boolean;
  /** Initial simulation speed. Default 1. */
  speed?: 1 | 10 | 60;
  /** "realtime" (default, browser timers) or "virtual" (driven by advance()/advanceReal(); for tests) or a custom Clock. */
  clock?: "realtime" | "virtual" | Clock;
  /** Layout persistence. Default: localStorage when available, else in-memory. */
  storage?: StorageLike | null;
}

/** The contract plus a few dev/test helpers (UI spikes should stick to `Backend`). */
export interface MockBackend extends Backend {
  readonly seed: number;
  /** Advance the ride by simS seconds of ride time (synchronously). With a virtual clock this also runs timers/subscriptions. */
  advance(simS: number): void;
  /** Virtual clock only: advance wall-clock time by ms (fires subscription timers, planner latency, ...). */
  advanceReal(ms: number): void;
  /** Finished hr_recovery blocks of this ride (final fit each). */
  pastRecoveries(): HrRecoveryState[];
  /** Stop all timers and drop all listeners. */
  dispose(): void;
}

const DRIVER_MS = 50;
const MAX_CATCHUP_MS = 2000;

export function createMockBackend(opts: MockBackendOptions = {}): MockBackend {
  const seed = (opts.seed ?? 42) >>> 0;
  const athlete: Athlete = { ...DEFAULT_ATHLETE, ...opts.athlete };
  const clock: Clock = opts.clock === "virtual" ? new VirtualClock() : opts.clock && opts.clock !== "realtime" ? opts.clock : realClock;
  const storage = opts.storage === undefined ? (browserStorage() ?? memoryStorage()) : (opts.storage ?? memoryStorage());
  const layouts = new LayoutStore(storage);
  const bus = new MetricBus();
  let speed: 1 | 10 | 60 = opts.speed ?? 1;

  const statusListeners = new Set<(s: RideStatus) => void>();
  const nudgeListeners = new Set<(n: CoachNudge) => void>();
  const subscriptions = new Set<Unsubscribe>();
  const pendingTimeouts = new Set<number>();
  let disposed = false;

  const status = (): RideStatus => ({ ...sim.status(), speed });
  const emitStatus = () => {
    if (!statusListeners.size) return;
    const s = status();
    for (const l of [...statusListeners]) l({ ...s });
  };

  let coach: Coach;
  const sim: RideSim = new RideSim(athlete, seed, bus, {
    statusChanged: emitStatus,
    evaluate: (t) => coach.evaluate(t),
    recoveryEnded: (st) => coach.onRecoveryEnded(st, sim.t),
    recoveryUnreachable: (st, step) => coach.onUnreachable(st, step, sim.t),
  });
  coach = new Coach(
    {
      features: () => sim.features(),
      usualTauS: sim.phys.usualTauS,
      apply: (pct) => sim.applyCoachPct(pct),
      endRecovery: (id) => {
        if (sim.inRecoveryOf(id)) sim.skipBlock();
      },
      emit: (n) => {
        for (const l of [...nudgeListeners]) l({ ...n });
      },
    },
    seed,
  );

  // ---------- driver: wall clock → fixed sim ticks ----------
  let driverId: number | null = null;
  let lastReal = 0;
  let debt = 0;
  let fires = 0;
  const stopDriver = () => {
    if (driverId !== null) clock.clearInterval(driverId);
    driverId = null;
  };
  const startDriver = () => {
    if (driverId !== null || disposed) return;
    lastReal = clock.now();
    debt = 0;
    driverId = clock.setInterval(() => {
      const now = clock.now();
      const dt = Math.min(MAX_CATCHUP_MS, Math.max(0, now - lastReal));
      lastReal = now;
      debt += (dt / 1000) * speed;
      while (debt >= SIM_DT - 1e-9 && sim.state === "riding") {
        sim.step();
        debt -= SIM_DT;
      }
      if (++fires % 10 === 0) emitStatus(); // 2 Hz heartbeat (t advances)
      if (sim.state !== "riding") {
        stopDriver();
        emitStatus();
      }
    }, DRIVER_MS);
  };

  // ---------- distributions cache ----------
  let mmpCache: { key: number; v: MmpCurve } | null = null;
  const best90 = syntheticMmp(athlete, 0.965);
  const allTime = syntheticMmp(athlete, 1.04);

  const ride: Backend["ride"] = {
    load(w) {
      stopDriver();
      coach.reset();
      sim.load(w);
      mmpCache = null;
      emitStatus();
    },
    start() {
      sim.start();
      if (sim.state === "riding") startDriver();
      emitStatus();
    },
    pause() {
      sim.pause();
      stopDriver();
    },
    resume() {
      if (sim.state === "paused") {
        sim.start();
        startDriver();
      }
    },
    stop() {
      sim.stop();
      stopDriver();
    },
    setSpeed(s) {
      speed = s;
      emitStatus();
    },
    adjust(pct) {
      sim.adjust(pct);
    },
    skipBlock() {
      sim.skipBlock();
    },
    current() {
      if (!sim.workout) return null;
      return { workout: structuredClone(sim.workout), compiled: sim.compiledView() as NonNullable<ReturnType<RideSim["compiledView"]>> };
    },
  };

  let rewriteCount = 0;
  const later = <T>(ms: number, f: () => T): Promise<T> =>
    new Promise((resolve, reject) => {
      const id = clock.setTimeout(() => {
        pendingTimeouts.delete(id);
        try {
          resolve(f());
        } catch (e) {
          reject(e);
        }
      }, ms);
      pendingTimeouts.add(id);
    });

  const backend: MockBackend = {
    seed,
    metrics: metricCatalog(athlete),
    zones: { power: powerZones(athlete.ftp), hr: hrZones(athlete.maxHr) },
    athlete: { ftp: athlete.ftp, weightKg: athlete.weightKg, maxHr: athlete.maxHr, restHr: athlete.restHr },

    subscribe(metric: MetricId, o: SubscribeOpts, cb: (s: Sample) => void): Unsubscribe {
      const rate = clamp(Number.isFinite(o.rateHz) ? o.rateHz : 1, 0.1, 10);
      const chain = createFilterChain(o.filter);
      let last: Sample | null = null;
      let dirty = false;
      let closed = false;
      const off = bus.listen(metric, (t, v, reset) => {
        if (reset) {
          chain.reset();
          last = { t: 0, v: null };
        } else last = { t, v: chain.push(t, v) };
        dirty = true;
      });
      const cur = bus.get(metric);
      if (cur) last = { t: cur.t, v: chain.push(cur.t, cur.v) };
      queueMicrotask(() => {
        if (closed) return;
        dirty = false;
        cb(last ? { ...last } : { t: sim.t, v: null });
      });
      const id = clock.setInterval(() => {
        if (!dirty || !last) return;
        dirty = false;
        cb({ ...last });
      }, 1000 / rate);
      const unsub = () => {
        if (closed) return;
        closed = true;
        off();
        clock.clearInterval(id);
        subscriptions.delete(unsub);
      };
      subscriptions.add(unsub);
      return unsub;
    },

    history(q: HistoryQuery): Bucket[] {
      return bus.history.query(q.metric, sim.t, Math.max(1, q.spanS), q.buckets, q.scale);
    },

    histogram(metric: MetricId, binWidth: number): Histogram {
      return histogramFrom(metric, bus.record.get(metric), binWidth, typicalDistribution(metric, athlete));
    },

    streams(metrics: MetricId[], fromS = 0): RideStreams {
      const from = Math.max(0, Math.floor(fromS));
      let to = from;
      const series: Partial<Record<MetricId, (number | null)[]>> = {};
      for (const m of metrics) {
        const rec = bus.record.get(m);
        series[m] = rec.slice(from).map((v) => (Number.isFinite(v) ? v : null));
        to = Math.max(to, rec.length);
      }
      return { fromS: from, toS: to, series };
    },

    recoveries: () => structuredClone(sim.pastRecoveries),

    heatmap(x: MetricId, y: MetricId, xBin: number, yBin: number): Heatmap2D {
      return heatmapFrom(x, y, bus.record.get(x), bus.record.get(y), xBin, yBin);
    },

    mmp(): MmpCurve {
      const key = Math.floor(sim.t);
      if (!mmpCache || mmpCache.key !== key) {
        mmpCache = { key, v: { durationsS: [...MMP_DURATIONS_S], today: mmpFrom(bus.record.get("power")), best90d: best90, allTime } };
      }
      const v = mmpCache.v;
      return { durationsS: [...v.durationsS], today: [...v.today], best90d: [...v.best90d], allTime: [...v.allTime] };
    },

    hrRecovery: () => sim.recoveryState(),

    onStatus(cb) {
      statusListeners.add(cb);
      cb(status());
      return () => {
        statusListeners.delete(cb);
      };
    },

    onNudge(cb) {
      nudgeListeners.add(cb);
      return () => {
        nudgeListeners.delete(cb);
      };
    },

    ride,

    compileWorkout: (w) => compileWorkout(w, { athlete, usualTauS: sim.phys.usualTauS }),
    parseDsl: (text, ftp) => parseDsl(text, ftp),
    printDsl: (nodes) => printDsl(nodes),
    snippets: () => seedSnippets(),

    coach: {
      respond: (callId, accept) => coach.respond(callId, accept),
      rate: (callId, rating, useful) => coach.rate(callId, rating, useful),
      decisions: () => coach.decisions(),
    },

    planner: {
      propose: (c: Checkin): Promise<Proposal> => later(plannerLatencyMs(c), () => proposeWorkout(c, athlete.ftp)),
      rewrite: (nodes: WorkoutNode[], intent: RewriteIntent): Promise<WorkoutNode[]> => {
        const snapshot = structuredClone(nodes);
        const salt = hashString(intent + JSON.stringify(snapshot.length)) + ++rewriteCount;
        const rng = new Rng(seed ^ salt);
        return later(700 + (salt % 200), () => rewriteNodes(snapshot, intent, athlete.ftp, rng));
      },
    },

    layouts: {
      list: () => layouts.list(),
      get: (id) => layouts.get(id),
      save: (l) => layouts.save(l),
      reset: (id) => layouts.reset(id),
    },

    advance(simS: number) {
      if (clock instanceof VirtualClock) {
        clock.advance((simS / speed) * 1000);
        return;
      }
      const n = Math.round(simS / SIM_DT);
      for (let i = 0; i < n && sim.state === "riding"; i++) sim.step();
      emitStatus();
    },

    advanceReal(ms: number) {
      if (!(clock instanceof VirtualClock)) throw new Error("advanceReal() needs { clock: 'virtual' }");
      clock.advance(ms);
    },

    pastRecoveries: () => structuredClone(sim.pastRecoveries),

    dispose() {
      disposed = true;
      stopDriver();
      for (const u of [...subscriptions]) u();
      for (const id of pendingTimeouts) clock.clearTimeout(id);
      pendingTimeouts.clear();
      statusListeners.clear();
      nudgeListeners.clear();
    },
  };

  const initial = opts.workout === undefined ? demoWorkout(athlete.ftp) : opts.workout;
  if (initial) sim.load(initial);
  if (opts.autoStart && initial) ride.start();
  return backend;
}
