import type { CompiledWorkout, FlatBlock, HrRecoveryState, MetricId, Phase, RideStatus, Sample, StepNode, Workout, WorkoutNode } from "./contract";
import { type Athlete, hrSteady, hrZones, type Physio, physioFor, powerForHr, powerZones, zoneIndex } from "./athlete";
import type { MetricBus } from "./bus";
import { dfaAlpha1, rmssd } from "./hrv";
import { buildHrRecoveryState, emptyRecoveryState } from "./hrRecovery";
import { Rng } from "./rng";
import { clamp, lerp } from "./util";
import { wbalStep } from "./wbal";
import { compileWorkout } from "./workout/compile";

/** Simulation step (s): trainer metrics at 4 Hz, HR-derived metrics every 4th tick (1 Hz). */
export const SIM_DT = 0.25;
const HR_FAST_W = 0.7;

export interface SimHooks {
  statusChanged(): void;
  /** called every 10 s of ride time while riding (coach evaluation) */
  evaluate(t: number): void;
  recoveryEnded(state: HrRecoveryState, step: StepNode): void;
  recoveryUnreachable(state: HrRecoveryState, step: StepNode): void;
}

export interface CoachFeatures {
  t: number;
  phase: Phase;
  blockIndex: number;
  blockElapsed: number;
  hrVsTypical: number | null;   // smoothed (20 s)
  hrDrift: number | null;
  decoupling: number | null;
  wbalPct: number;
  cadence10s: number | null;
  power3s: number | null;
  target: number;
  compliance: number | null;
}

interface RecoveryCtx {
  step: StepNode & { end: { kind: "hr_recovery" } };
  startedT: number;
  observed: Sample[];
  priorHrInf: number;
  lowered: boolean;
  handled: boolean;
  unreachableSince: number | null;
  state: HrRecoveryState;
  stateT: number;
}

function collectSteps(nodes: readonly WorkoutNode[], out: Map<string, StepNode>): Map<string, StepNode> {
  for (const n of nodes) {
    if (n.type === "step") out.set(n.id, n);
    else collectSteps(n.children, out);
  }
  return out;
}

/**
 * Deterministic ride simulator: an ERG trainer + a physiologically plausible athlete.
 * Advances in fixed SIM_DT ticks; publishes every metric to the MetricBus at its source rate.
 */
export class RideSim {
  readonly phys: Physio;
  private rng: Rng;

  workout: Workout | null = null;
  compiled: CompiledWorkout | null = null;
  private steps = new Map<string, StepNode>();
  private blockScale: number[] = [];
  private aiChanged: boolean[] = [];

  state: RideStatus["state"] = "idle";
  private tick = 0;
  blockIdx = 0;
  private blockStartT = 0;
  private blockKj = 0;
  private blockPowSum = 0;
  private blockPowN = 0;
  manualOffsetPct = 0;
  mode: RideStatus["mode"] = "erg";
  trainer: RideStatus["trainer"] = "connected";
  hrSensor: RideStatus["hrSensor"] = "connected";

  // trainer
  private ergCmd = 0;
  private ar = 0;
  private spikeLeft = 0;
  private spikeMag = 0;
  private target = 0;
  private blipStart = 0;
  private blipEnd = 0;
  // athlete
  private cadAr = 0;
  private cadence = 0;
  private vMs = 0;
  private distKm = 0;
  private delay = new Float64Array(16);
  private delayIdx = 0;
  private pEma = 0;
  private hrF = 0;
  private hrS = 0;
  private typF = 0;
  private typS = 0;
  private drift = 0;
  private dayOffset = 0;
  private wbal = 0;
  private wExpended = 0;
  private hrDropLeft = 0;
  private lastHr: number | null = null;
  private hrCtrl = 0;
  private intensityEma = 0;
  // RR
  private nextBeatIn = 0.8;
  private rrAr = 0;
  private rrPrevU = 0;
  private beats: number[] = [];
  // metric accumulators
  private powTicks = new Float64Array(120).fill(NaN);
  private secPow: number[] = [];
  private roll30: number[] = [];
  private sum4 = 0;
  private n4 = 0;
  private np: number | null = null;
  private kj = 0;
  private complianceEligible = 0;
  private complianceOk = 0;
  private resid: number[] = [];
  private driftBaseline: number | null = null;
  private pre = { p: [0], h: [0], n: [0] };
  private hrVsTypEma: number | null = null;
  private recovery: RecoveryCtx | null = null;
  private lastRecovery: HrRecoveryState = emptyRecoveryState();
  readonly pastRecoveries: HrRecoveryState[] = [];

  private readonly pz: ReturnType<typeof powerZones>;
  private readonly hz: ReturnType<typeof hrZones>;

  constructor(readonly athlete: Athlete, private readonly seed: number, private readonly bus: MetricBus, private readonly hooks: SimHooks) {
    this.phys = physioFor(athlete);
    this.rng = new Rng(seed);
    this.pz = powerZones(athlete.ftp);
    this.hz = hrZones(athlete.maxHr);
  }

  get t(): number {
    return this.tick * SIM_DT;
  }

  load(w: Workout): void {
    this.workout = structuredClone(w);
    this.compiled = compileWorkout(this.workout, { athlete: this.athlete, usualTauS: this.phys.usualTauS });
    this.steps = collectSteps(this.workout.nodes, new Map());
    this.blockScale = this.compiled.blocks.map(() => 1);
    this.aiChanged = this.compiled.blocks.map(() => false);
    this.resetRide();
  }

  private resetRide(): void {
    this.rng = new Rng(this.seed);
    this.state = "idle";
    this.tick = 0;
    this.blockIdx = 0;
    this.blockStartT = 0;
    this.blockKj = 0;
    this.blockPowSum = 0;
    this.blockPowN = 0;
    this.manualOffsetPct = 0;
    this.trainer = "connected";
    this.hrSensor = "connected";
    this.ergCmd = 0;
    this.ar = 0;
    this.spikeLeft = 0;
    this.blipStart = this.rng.uniform(600, 1100);
    this.blipEnd = this.blipStart + this.rng.uniform(3, 6);
    this.dayOffset = this.rng.uniform(-1, 4);
    this.cadAr = 0;
    this.cadence = 0;
    this.vMs = 0;
    this.distKm = 0;
    this.delay.fill(0);
    this.delayIdx = 0;
    this.pEma = 0;
    const r = 8; // pre-ride HR slightly above rest
    this.hrF = HR_FAST_W * r;
    this.hrS = (1 - HR_FAST_W) * r;
    this.typF = this.hrF;
    this.typS = this.hrS;
    this.drift = 0;
    this.wbal = this.phys.wPrime;
    this.wExpended = 0;
    this.hrDropLeft = 0;
    this.lastHr = null;
    this.hrCtrl = 0;
    this.intensityEma = 0;
    this.nextBeatIn = 0.8;
    this.rrAr = 0;
    this.rrPrevU = 0;
    this.beats = [];
    this.powTicks.fill(NaN);
    this.secPow = [];
    this.roll30 = [];
    this.sum4 = 0;
    this.n4 = 0;
    this.np = null;
    this.kj = 0;
    this.complianceEligible = 0;
    this.complianceOk = 0;
    this.resid = [];
    this.driftBaseline = null;
    this.pre = { p: [0], h: [0], n: [0] };
    this.hrVsTypEma = null;
    this.recovery = null;
    this.lastRecovery = emptyRecoveryState();
    this.pastRecoveries.length = 0;
    this.bus.reset();
    const first = this.compiled?.blocks[0];
    if (first) this.enterBlock(0, 0, first);
  }

  // ---------- control ----------

  start(): void {
    if (!this.compiled || this.compiled.blocks.length === 0) return;
    if (this.state === "finished") this.resetRide();
    if (this.state === "idle" || this.state === "paused") {
      this.state = "riding";
      this.hooks.statusChanged();
    }
  }

  pause(): void {
    if (this.state === "riding") {
      this.state = "paused";
      this.hooks.statusChanged();
    }
  }

  stop(): void {
    if (this.state === "riding" || this.state === "paused") this.finish();
  }

  adjust(pct: number): void {
    this.manualOffsetPct = Math.round((this.manualOffsetPct + pct) * 10) / 10;
    this.hooks.statusChanged();
  }

  skipBlock(): void {
    if (this.state === "riding" || this.state === "paused") this.advanceBlock(this.t);
  }

  /** Coach: scale remaining work blocks (from the current one if it is work). */
  applyCoachPct(pct: number): void {
    const c = this.compiled;
    if (!c) return;
    for (let i = this.blockIdx; i < c.blocks.length; i++) {
      if ((c.blocks[i] as FlatBlock).phase !== "work") continue;
      this.blockScale[i] = (this.blockScale[i] as number) * (1 + pct / 100);
      this.aiChanged[i] = true;
    }
  }

  get currentBlock(): FlatBlock | undefined {
    return this.compiled?.blocks[this.blockIdx];
  }

  get currentStep(): StepNode | undefined {
    const b = this.currentBlock;
    return b ? this.steps.get(b.nodeId) : undefined;
  }

  /** Compiled workout as currently being ridden (coach changes applied, aiChanged flags set). */
  compiledView(): CompiledWorkout | null {
    const c = this.compiled;
    if (!c) return null;
    const out = structuredClone(c);
    out.blocks.forEach((b, i) => {
      const k = this.blockScale[i] ?? 1;
      if (k !== 1) {
        b.powerFrom = Math.round(b.powerFrom * k * 10) / 10;
        b.powerTo = Math.round(b.powerTo * k * 10) / 10;
      }
      if (this.aiChanged[i]) b.aiChanged = true;
    });
    return out;
  }

  status(): RideStatus {
    const b = this.currentBlock;
    return {
      state: this.state,
      t: this.t,
      speed: 1,
      mode: this.mode,
      trainer: this.trainer,
      hrSensor: this.hrSensor,
      blockIndex: this.blockIdx,
      phase: b?.phase ?? "free",
      manualOffsetPct: this.manualOffsetPct,
    };
  }

  features(): CoachFeatures {
    const b = this.currentBlock;
    return {
      t: this.t,
      phase: b?.phase ?? "free",
      blockIndex: this.blockIdx,
      blockElapsed: this.t - this.blockStartT,
      hrVsTypical: this.hrVsTypEma,
      hrDrift: this.bus.value("hr_drift"),
      decoupling: this.bus.value("decoupling"),
      wbalPct: (this.wbal / this.phys.wPrime) * 100,
      cadence10s: this.cadAvg(40),
      power3s: this.avgTicks(12),
      target: this.target,
      compliance: this.bus.value("compliance"),
    };
  }

  /** Live HR recovery state (fresh fit if new samples arrived); last finished one when not in a recovery block. */
  recoveryState(): HrRecoveryState {
    const r = this.recovery;
    if (!r) return structuredClone(this.lastRecovery);
    if (r.stateT !== this.t) this.refit(r);
    return structuredClone(r.state);
  }

  // ---------- simulation ----------

  private finish(): void {
    if (this.recovery) this.endRecovery();
    this.state = "finished";
    this.hooks.statusChanged();
  }

  private enterBlock(idx: number, t: number, b: FlatBlock): void {
    this.blockIdx = idx;
    this.blockStartT = t;
    this.blockKj = 0;
    this.blockPowSum = 0;
    this.blockPowN = 0;
    this.hrCtrl = 0;
    const step = this.steps.get(b.nodeId);
    this.mode = step?.target.kind === "free" ? "free" : "erg";
    if (step && step.end.kind === "hr_recovery") {
      const recP = b.powerFrom * (1 + this.manualOffsetPct / 100);
      const offset = clamp(this.hrVsTypEma ?? 0, -5, 10);
      const ctx: RecoveryCtx = {
        step: step as RecoveryCtx["step"],
        startedT: t,
        observed: [],
        priorHrInf: hrSteady(recP, this.athlete, this.phys) + offset,
        lowered: false,
        handled: false,
        unreachableSince: null,
        state: emptyRecoveryState(),
        stateT: -1,
      };
      ctx.state = { ...emptyRecoveryState(), active: true, startedT: t, peakT: t, targetBpm: step.end.targetBpm, minS: step.end.minS, maxS: step.end.maxS };
      this.recovery = ctx;
    }
  }

  private endRecovery(): void {
    const r = this.recovery;
    if (!r) return;
    this.refit(r);
    const final = { ...structuredClone(r.state), active: false };
    this.lastRecovery = final;
    this.pastRecoveries.push(final);
    this.recovery = null;
    this.hooks.recoveryEnded(structuredClone(final), r.step);
  }

  private advanceBlock(t: number): void {
    const c = this.compiled;
    if (!c) return;
    if (this.recovery) this.endRecovery();
    const next = this.blockIdx + 1;
    if (next >= c.blocks.length) {
      this.finish();
      return;
    }
    this.enterBlock(next, t, c.blocks[next] as FlatBlock);
    this.hooks.statusChanged();
  }

  private blockDone(t: number): boolean {
    const b = this.currentBlock;
    const s = this.currentStep;
    if (!b || !s) return true;
    const el = t - this.blockStartT;
    switch (s.end.kind) {
      case "time":
      case "lap":
        return el >= b.durS - 1e-9;
      case "kj":
        return this.blockKj >= s.end.kj;
      case "hr_recovery":
        return el >= s.end.maxS || (el >= s.end.minS && this.lastHr !== null && this.lastHr <= s.end.targetBpm);
    }
  }

  private computeTarget(b: FlatBlock, s: StepNode, el: number): number {
    const ftp = this.athlete.ftp;
    let frac = b.durS > 0 ? clamp(el / b.durS, 0, 1) : 0;
    if (s.end.kind === "kj") frac = clamp(this.blockKj / s.end.kj, 0, 1);
    let base = lerp(b.powerFrom, b.powerTo, frac);
    if (s.target.kind === "hr") {
      if (this.lastHr !== null) this.hrCtrl = clamp(this.hrCtrl + 0.4 * (s.target.bpm - this.lastHr) * SIM_DT, -0.3 * ftp, 0.3 * ftp);
      base = clamp(powerForHr(s.target.bpm, this.athlete, this.phys) + this.hrCtrl, 0.3 * ftp, 1.1 * ftp);
    } else if (s.target.kind === "free") {
      base = 0.55 * ftp * (1 + 0.1 * Math.sin(this.t / 90) + 0.05 * Math.sin(this.t / 23));
    }
    let k = (this.blockScale[this.blockIdx] ?? 1) * (1 + this.manualOffsetPct / 100);
    if (this.recovery?.lowered) k *= 0.75;
    return Math.max(0, base * k);
  }

  private avgTicks(n: number): number | null {
    let s = 0;
    let c = 0;
    for (let i = 0; i < n; i++) {
      const v = this.powTicks[(this.tick - i + 1200) % 120] as number;
      if (Number.isFinite(v)) {
        s += v;
        c++;
      }
    }
    return c ? s / c : null;
  }

  private cadTicks = new Float64Array(40).fill(NaN);
  private cadAvg(n: number): number | null {
    let s = 0;
    let c = 0;
    for (let i = 0; i < Math.min(n, 40); i++) {
      const v = this.cadTicks[(this.tick - i + 400) % 40] as number;
      if (Number.isFinite(v)) {
        s += v;
        c++;
      }
    }
    return c ? s / c : null;
  }

  /** Advance one SIM_DT tick (no-op unless riding). */
  step(): void {
    if (this.state !== "riding" || !this.compiled) return;
    const a = this.athlete;
    const ph = this.phys;
    const rng = this.rng;
    this.tick++;
    const t = this.t;

    // ----- block transitions -----
    let guard = 0;
    while (this.blockDone(t) && guard++ < 1000) {
      this.advanceBlock(t);
      if (this.state !== "riding") return;
    }
    const b = this.currentBlock as FlatBlock;
    const s = this.currentStep as StepNode;
    const el = t - this.blockStartT;

    // ----- trainer -----
    const prevTrainer = this.trainer;
    const trainerOk = !(t >= this.blipStart && t < this.blipEnd);
    this.trainer = trainerOk ? "connected" : "connecting";
    this.target = this.computeTarget(b, s, el);
    this.ergCmd += (this.target - this.ergCmd) * (1 - Math.exp(-SIM_DT / 1.2));
    this.ar = 0.9 * this.ar + rng.normal(0, 0.011);
    if (this.spikeLeft > 0) this.spikeLeft--;
    else if (rng.chance(0.0025)) {
      this.spikeLeft = rng.int(1, 6);
      this.spikeMag = rng.chance(0.75) ? rng.uniform(0.15, 0.4) : -rng.uniform(0.2, 0.4);
    }
    const spike = this.spikeLeft > 0 ? this.spikeMag : 0;
    const actual = Math.max(0, this.ergCmd * (1 + this.ar + spike + rng.normal(0, 0.012)));
    const measured = trainerOk ? Math.round(actual) : null;

    // ----- energy -----
    this.wbal = wbalStep(this.wbal, actual, SIM_DT, ph.cp, ph.wPrime);
    if (actual > ph.cp) this.wExpended += (actual - ph.cp) * SIM_DT;
    const wFrac = clamp(this.wbal / ph.wPrime, 0, 1);
    this.kj += (actual * SIM_DT) / 1000;
    this.blockKj += (actual * SIM_DT) / 1000;
    if (measured !== null) {
      this.blockPowSum += measured;
      this.blockPowN++;
    }
    this.powTicks[this.tick % 120] = measured ?? NaN;

    // ----- cadence -----
    const I = actual / a.ftp;
    let cadPref = s.cadence ? (s.cadence.lo + s.cadence.hi) / 2 : 87 + 8 * clamp((I - 0.6) / 0.6, 0, 1);
    cadPref -= 2.5 * (t / 3600) + 5 * (1 - wFrac);
    this.cadAr = 0.97 * this.cadAr + rng.normal(0, 0.35);
    const cadTarget = actual < 5 ? 0 : cadPref + this.cadAr;
    this.cadence += (cadTarget - this.cadence) * (1 - Math.exp(-SIM_DT / 1.5));
    const cadMeasured = trainerOk ? Math.round(this.cadence * 10) / 10 : null;
    this.cadTicks[this.tick % 40] = cadMeasured ?? NaN;

    // ----- speed (flat road, rider + bike 81 kg) -----
    const m = a.weightKg + 9;
    const pw = actual * 0.97;
    let v = Math.max(1, this.vMs);
    for (let i = 0; i < 3; i++) {
      const f = 0.5 * 1.2 * 0.32 * v * v * v + 0.004 * m * 9.81 * v - pw;
      const df = 1.5 * 1.2 * 0.32 * v * v + 0.004 * m * 9.81;
      v = Math.max(0, v - f / df);
    }
    this.vMs += (v - this.vMs) * (1 - Math.exp(-SIM_DT / 3));
    this.distKm += (this.vMs * SIM_DT) / 1000;

    // ----- heart -----
    const pDelayed = this.delay[this.delayIdx] as number;
    this.delay[this.delayIdx] = actual;
    this.delayIdx = (this.delayIdx + 1) % this.delay.length;
    this.pEma += (pDelayed - this.pEma) * (1 - Math.exp(-SIM_DT / 4));
    this.intensityEma += (this.pEma / ph.cp - this.intensityEma) * (1 - Math.exp(-SIM_DT / 60));
    this.drift += (SIM_DT / 60) * 0.12 * clamp((this.pEma / a.ftp - 0.4) / 0.6, 0, 1.5);
    const fatigue = 1 + 0.2 * Math.min(2.5, this.wExpended / ph.wPrime) + 0.3 * (1 - wFrac);
    const ssTyp = hrSteady(this.pEma, a, ph) - a.restHr;
    const ss = Math.min(a.maxHr - 1, hrSteady(this.pEma, a, ph) + this.drift + this.dayOffset + 4 * (1 - wFrac)) - a.restHr;
    const relax = (x: number, target: number, tauUp: number, tauDown: number) =>
      x + (target - x) * (1 - Math.exp(-SIM_DT / (target > x ? tauUp : tauDown)));
    // two-component HR kinetics: fast (70 %) + slow (30 %); recovery slows with fatigue
    this.hrF = relax(this.hrF, HR_FAST_W * ss, 20, 32 * fatigue);
    this.hrS = relax(this.hrS, (1 - HR_FAST_W) * ss, 60, 100 * fatigue);
    this.typF = relax(this.typF, HR_FAST_W * ssTyp, 20, 32);
    this.typS = relax(this.typS, (1 - HR_FAST_W) * ssTyp, 60, 100);
    const hrTrue = a.restHr + this.hrF + this.hrS;
    const hrTyp = a.restHr + this.typF + this.typS;

    // ----- RR intervals -----
    this.nextBeatIn -= SIM_DT;
    while (this.nextBeatIn <= 0) {
      const rr = this.nextRr(hrTrue);
      const beatT = t + this.nextBeatIn;
      if (this.hrDropLeft <= 0) {
        this.bus.publish("rr", Math.max(0, beatT), Math.round(rr));
        this.beats.push(rr);
        if (this.beats.length > 400) this.beats.splice(0, this.beats.length - 300);
      }
      this.nextBeatIn += rr / 1000;
    }

    // ----- 4 Hz publish -----
    const p3 = this.avgTicks(12);
    const P = (id: MetricId, val: number | null) => this.bus.publish(id, t, val);
    P("power", measured);
    P("power_3s", p3);
    P("power_10s", this.avgTicks(40));
    P("power_30s", this.avgTicks(120));
    P("power_lap", this.blockPowN ? this.blockPowSum / this.blockPowN : null);
    P("w_per_kg", p3 === null ? null : Math.round((p3 / a.weightKg) * 100) / 100);
    P("pct_ftp", p3 === null ? null : (p3 / a.ftp) * 100);
    P("target_power", Math.round(this.target));
    P("delta_target", p3 === null ? null : p3 - this.target);
    P("power_zone", p3 === null ? null : zoneIndex(this.pz, p3) + 1);
    P("wbal_pct", (this.wbal / ph.wPrime) * 100);
    P("kj", this.kj);
    P("cadence", cadMeasured);
    P("target_cadence", s.cadence ? (s.cadence.lo + s.cadence.hi) / 2 : null);
    P("speed", trainerOk ? this.vMs * 3.6 : null);
    P("distance", this.distKm);
    P("elapsed", t);
    P("block_index", this.blockIdx + 1);
    P("block_remaining", this.blockRemaining(b, s, el));

    // ----- 1 Hz -----
    if (this.tick % 4 === 0) this.perSecond(t, b, s, el, hrTrue, hrTyp, p3, trainerOk);

    if (prevTrainer !== this.trainer) this.hooks.statusChanged();
    if (this.tick % 40 === 0) this.hooks.evaluate(t);
  }

  private nextRr(hr: number): number {
    const rng = this.rng;
    const a = this.athlete;
    const meanRr = 60000 / hr;
    const sigma = Math.max(3.5, 50 * Math.exp(-(hr - a.restHr) / 35));
    const I = this.intensityEma;
    const alpha =
      I <= 0.5 ? 1.15 : I <= 0.75 ? lerp(1.15, 0.75, (I - 0.5) / 0.25) : I <= 1 ? lerp(0.75, 0.4, (I - 0.75) / 0.25) : I <= 1.15 ? lerp(0.4, 0.25, (I - 1) / 0.15) : 0.25;
    const phi = 0.85;
    this.rrAr = phi * this.rrAr + Math.sqrt(1 - phi * phi) * rng.normal();
    const white = rng.normal();
    const u = rng.normal();
    const anti = (u - this.rrPrevU) / Math.SQRT2;
    this.rrPrevU = u;
    let z: number;
    if (alpha >= 0.5) {
      const wa = clamp((alpha - 0.5) / 0.75, 0, 1);
      z = Math.sqrt(wa) * this.rrAr + Math.sqrt(1 - wa) * white;
    } else {
      const wb = clamp((0.5 - alpha) / 0.4, 0, 1);
      z = Math.sqrt(1 - wb) * white + Math.sqrt(wb) * anti;
    }
    return meanRr + sigma * z;
  }

  private blockRemaining(b: FlatBlock, s: StepNode, el: number): number {
    switch (s.end.kind) {
      case "time":
      case "lap":
        return Math.max(0, b.durS - el);
      case "kj":
        return Math.max(0, ((s.end.kj - this.blockKj) * 1000) / Math.max(1, this.target));
      case "hr_recovery": {
        const r = this.recovery?.state.readyInS;
        if (r) return Math.max(0, r.mean - (this.t - (this.recovery as RecoveryCtx).stateT));
        return Math.max(0, s.end.maxS - el);
      }
    }
  }

  private perSecond(t: number, b: FlatBlock, s: StepNode, el: number, hrTrue: number, hrTyp: number, p3: number | null, trainerOk: boolean): void {
    const a = this.athlete;
    const rng = this.rng;
    const P = (id: MetricId, val: number | null) => this.bus.publish(id, t, val);

    // HR strap with occasional dropouts
    const prevSensor = this.hrSensor;
    if (this.hrDropLeft > 0) this.hrDropLeft--;
    else if (t > 60 && rng.chance(1 / 420)) this.hrDropLeft = rng.int(2, 6);
    this.hrSensor = this.hrDropLeft > 0 ? "lost" : "connected";
    const hr = this.hrDropLeft > 0 ? null : Math.round(hrTrue + rng.normal(0, 0.6));
    if (hr !== null) this.lastHr = hr;
    P("hr", hr);
    P("hr_zone", hr === null ? null : zoneIndex(this.hz, hr) + 1);
    P("hr_pct_max", hr === null ? null : (hr / a.maxHr) * 100);
    const vsTyp = hr === null ? null : hr - hrTyp;
    if (vsTyp !== null) this.hrVsTypEma = this.hrVsTypEma === null ? vsTyp : this.hrVsTypEma + (vsTyp - this.hrVsTypEma) * (1 - Math.exp(-1 / 20));
    P("hr_vs_typical", vsTyp);

    // 1 s power for NP / decoupling
    const sec = Math.round(t) - 1;
    let ps = 0;
    let pn = 0;
    for (let i = 0; i < 4; i++) {
      const v = this.powTicks[(this.tick - i + 1200) % 120] as number;
      if (Number.isFinite(v)) {
        ps += v;
        pn++;
      }
    }
    const p1 = pn ? ps / pn : NaN;
    this.secPow[sec] = p1;
    this.roll30.push(Number.isFinite(p1) ? p1 : 0);
    if (this.roll30.length > 30) this.roll30.shift();
    if (this.roll30.length === 30) {
      const avg = this.roll30.reduce((x, y) => x + y, 0) / 30;
      this.sum4 += avg ** 4;
      this.n4++;
      this.np = Math.pow(this.sum4 / this.n4, 0.25);
    }
    const IF = this.np === null ? null : this.np / a.ftp;
    P("np", this.np);
    P("if", IF);
    P("tss", this.np === null || IF === null ? null : ((t * this.np * IF) / (a.ftp * 3600)) * 100);

    // drift: residual vs typical model, last 5 min vs minutes 5–15
    this.resid[sec] = vsTyp ?? NaN;
    if (this.driftBaseline === null && t >= 900) this.driftBaseline = meanRange(this.resid, 300, 900);
    P("hr_drift", this.driftBaseline === null ? null : meanRange(this.resid, Math.max(0, sec - 299), sec + 1) - this.driftBaseline);

    // Pa:HR decoupling (first vs second half of the ride so far)
    const both = Number.isFinite(p1) && hr !== null;
    const L = this.pre.p.length - 1;
    this.pre.p.push((this.pre.p[L] as number) + (both ? p1 : 0));
    this.pre.h.push((this.pre.h[L] as number) + (both ? (hr as number) : 0));
    this.pre.n.push((this.pre.n[L] as number) + (both ? 1 : 0));
    const T = this.pre.p.length - 1;
    if (T >= 600) {
      const h = Math.floor(T / 2);
      const ef = (i0: number, i1: number) => {
        const n = (this.pre.n[i1] as number) - (this.pre.n[i0] as number);
        const ph = (this.pre.h[i1] as number) - (this.pre.h[i0] as number);
        return n && ph ? ((this.pre.p[i1] as number) - (this.pre.p[i0] as number)) / ph : NaN;
      };
      const e1 = ef(0, h);
      const e2 = ef(h, T);
      P("decoupling", Number.isFinite(e1) && Number.isFinite(e2) && e1 > 0 ? ((e1 - e2) / e1) * 100 : null);
    } else P("decoupling", null);

    // HRV
    const last60: number[] = [];
    let acc = 0;
    for (let i = this.beats.length - 1; i >= 0 && acc < 60000; i--) {
      last60.push(this.beats[i] as number);
      acc += this.beats[i] as number;
    }
    last60.reverse();
    P("rmssd", this.hrDropLeft > 0 ? null : rmssd(last60));
    P("dfa_a1", this.hrDropLeft > 0 ? null : dfaAlpha1(this.beats.slice(-120)));

    // compliance: share of ERG seconds within ±5 % of target (3 s power), settle 5 s per block
    const ergTarget = s.target.kind !== "free" && s.target.kind !== "hr";
    if (ergTarget && trainerOk && el >= 5 && this.target > 0 && p3 !== null) {
      this.complianceEligible++;
      if (Math.abs(p3 - this.target) <= 0.05 * this.target) this.complianceOk++;
    }
    P("compliance", this.complianceEligible ? (this.complianceOk / this.complianceEligible) * 100 : null);

    // remaining (expected)
    const c = this.compiled as CompiledWorkout;
    let rem = this.blockRemaining(b, s, el);
    for (let i = this.blockIdx + 1; i < c.blocks.length; i++) rem += (c.blocks[i] as FlatBlock).durS;
    P("remaining", rem);

    // HR recovery tracking
    const r = this.recovery;
    if (r) {
      r.observed.push({ t, v: hr });
      if (Math.round(t) % 2 === 0) this.refit(r);
      const st = r.state;
      // act on "unreachable" only once it has persisted for 20 s (early fits wobble)
      r.unreachableSince = st.fit && st.unreachable ? (r.unreachableSince ?? t) : null;
      if (r.unreachableSince !== null && !r.handled && t - r.unreachableSince >= 20 && t - st.peakT >= 45) {
        r.handled = true;
        if (r.step.end.onUnreachable === "lower_power") {
          r.lowered = true;
          r.priorHrInf -= 6;
        } else if (r.step.end.onUnreachable === "ask_coach") {
          this.hooks.recoveryUnreachable(structuredClone(st), r.step);
        }
      }
    }

    if (prevSensor !== this.hrSensor) this.hooks.statusChanged();
  }

  private refit(r: RecoveryCtx): void {
    const e = r.step.end;
    r.state = buildHrRecoveryState({
      startedT: r.startedT,
      minS: e.minS,
      maxS: e.maxS,
      targetBpm: e.targetBpm,
      observed: r.observed,
      now: this.t,
      usualTauS: this.phys.usualTauS,
      priorHrInf: r.priorHrInf,
    });
    r.stateT = this.t;
  }

  /** True while in a recovery block whose id matches (used by coach ask_coach accept). */
  inRecoveryOf(stepId: string): boolean {
    return this.recovery?.step.id === stepId;
  }
}

function meanRange(xs: readonly number[], i0: number, i1: number): number {
  let s = 0;
  let n = 0;
  for (let i = i0; i < i1; i++) {
    const v = xs[i];
    if (v !== undefined && Number.isFinite(v)) {
      s += v;
      n++;
    }
  }
  return n ? s / n : 0;
}
