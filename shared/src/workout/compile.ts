import type { CompiledWorkout, FlatBlock, Phase, StepNode, ValidationIssue, Workout, WorkoutNode } from "../contract";
import { type Athlete, DEFAULT_ATHLETE, hrSteady, physioFor, powerZones, zoneIndex } from "../athlete";
import { wbalSeries } from "../wbal";
import { isWorkStep, type Offset, targetPct, targetWatts } from "./model";

/** Trainer hardware limit (Tacx NEO 2T). */
export const TRAINER_MAX_W = 2200;
/** Expected duration and range assumed for `lap` (open-ended, user presses lap) steps. */
export const LAP_EXPECTED_S = 300;
export const LAP_RANGE_S: [number, number] = [30, 1800];

interface Raw {
  step: StepNode;
  path: number[];
  iteration: number[];
  off: Offset;
}

function flatten(nodes: readonly WorkoutNode[], ftp: number, out: Raw[], issues: ValidationIssue[]): void {
  nodes.forEach((n, i) => visit(n, i, [], [], { watts: 0, pct: 0 }, ftp, out, issues));
}

function visit(n: WorkoutNode, index: number, path: number[], iteration: number[], off: Offset, ftp: number, out: Raw[], issues: ValidationIssue[]): void {
  const p = [...path, index];
  if (n.type === "step") {
    // progression applies to work steps only (rests stay put)
    out.push({ step: n, path: p, iteration, off: isWorkStep(n, ftp) ? off : { watts: 0, pct: 0 } });
    return;
  }
  if (!Number.isInteger(n.count) || n.count < 1) {
    issues.push({ severity: "error", nodeId: n.id, message: `Repeat count must be a whole number ≥ 1 (got ${n.count})` });
    return;
  }
  if (n.children.length === 0) {
    issues.push({ severity: "error", nodeId: n.id, message: "Repeat has no steps" });
    return;
  }
  for (let r = 0; r < n.count; r++) {
    const o: Offset = {
      watts: off.watts + r * (n.progression?.wattsPerRep ?? 0),
      pct: off.pct + r * (n.progression?.pctPerRep ?? 0),
    };
    const last = r === n.count - 1;
    n.children.forEach((c, ci) => {
      if (last && n.skipLastRest && n.children.length > 1 && ci === n.children.length - 1) return;
      visit(c, ci, p, [...iteration, r], o, ftp, out, issues);
    });
  }
}

/** Classify phases: warmup before the first work block, cooldown after the last, recovery between. */
function classifyPhases(raws: Raw[], ftp: number): Phase[] {
  const isWork = raws.map((r) => isWorkStep(r.step, ftp));
  let first = isWork.indexOf(true);
  let last = isWork.lastIndexOf(true);
  // an all-easy ride: treat the highest-intensity blocks as the "work"
  if (first < 0 && raws.length) {
    const pcts = raws.map((r) => targetPct(r.step.target, ftp));
    const max = Math.max(...pcts);
    pcts.forEach((p, i) => (isWork[i] = p >= max * 0.95 && raws[i]?.step.target.kind !== "free"));
    first = isWork.indexOf(true);
    last = isWork.lastIndexOf(true);
  }
  return raws.map((r, i) => {
    const label = (r.step.label ?? "").toLowerCase();
    if (r.step.target.kind === "free") return "free";
    if (label.includes("warm")) return "warmup";
    if (label.includes("cool")) return "cooldown";
    if (r.step.end.kind === "hr_recovery") return "recovery";
    if (isWork[i]) return "work";
    if (first >= 0 && i < first) return "warmup";
    if (last >= 0 && i > last) return "cooldown";
    return "recovery";
  });
}

export interface CompileOptions {
  athlete?: Athlete;
  /** usual HR recovery τ for hr_recovery duration estimates */
  usualTauS?: number;
}

/**
 * Flatten a workout tree into timed blocks and compute summary + validation.
 * Pure and deterministic. Durations of open-ended blocks are estimates (see durRangeS).
 */
export function compileWorkout(w: Workout, opts: CompileOptions = {}): CompiledWorkout {
  const athlete = { ...(opts.athlete ?? DEFAULT_ATHLETE), ftp: w.ftp };
  const phys = physioFor(athlete);
  const usualTau = opts.usualTauS ?? phys.usualTauS;
  const ftp = w.ftp;
  const issues: ValidationIssue[] = [];
  if (!(ftp > 0)) issues.push({ severity: "error", message: "FTP must be positive" });

  const raws: Raw[] = [];
  flatten(w.nodes, ftp, raws, issues);
  const phases = classifyPhases(raws, ftp);

  const blocks: FlatBlock[] = [];
  let t = 0;
  let hrExp = athlete.restHr + 10; // expected HR trace (single time constant), for hr_recovery estimates
  const rangeTotal: [number, number] = [0, 0];
  const maxAllowed = Math.min(TRAINER_MAX_W, 4 * ftp);

  raws.forEach((r, i) => {
    const s = r.step;
    let [pf, pt] = targetWatts(s.target, ftp, r.off, athlete);
    let durS = 0;
    let durRangeS: [number, number] | undefined;
    switch (s.end.kind) {
      case "time":
        durS = s.end.s;
        if (!(durS > 0)) issues.push({ severity: "error", nodeId: s.id, message: "Duration must be > 0" });
        break;
      case "kj": {
        const avg = Math.max(1, (pf + pt) / 2);
        durS = (s.end.kj * 1000) / avg;
        if (!(s.end.kj > 0)) issues.push({ severity: "error", nodeId: s.id, message: "kJ target must be > 0" });
        break;
      }
      case "lap":
        durS = LAP_EXPECTED_S;
        durRangeS = [...LAP_RANGE_S];
        break;
      case "hr_recovery": {
        const e = s.end;
        if (e.minS > e.maxS) issues.push({ severity: "error", nodeId: s.id, message: "HR recovery: min duration exceeds max" });
        if (e.targetBpm <= athlete.restHr || e.targetBpm >= athlete.maxHr)
          issues.push({ severity: "error", nodeId: s.id, message: `HR recovery target ${e.targetBpm} bpm is outside ${athlete.restHr}–${athlete.maxHr}` });
        const hrInf = hrSteady(pf, athlete, phys);
        let est: number;
        if (hrInf >= e.targetBpm - 1) {
          est = e.maxS;
          issues.push({
            severity: "warning",
            nodeId: s.id,
            message: `HR < ${e.targetBpm} is likely unreachable at ${Math.round(pf)} W (steady HR ≈ ${Math.round(hrInf)}); block will run to its max`,
          });
        } else if (hrExp <= e.targetBpm) {
          est = e.minS;
        } else {
          est = usualTau * Math.log((hrExp - hrInf) / (e.targetBpm - hrInf));
        }
        durS = Math.min(e.maxS, Math.max(e.minS, est));
        durRangeS = [Math.min(e.minS, e.maxS), Math.max(e.minS, e.maxS)];
        break;
      }
    }
    durS = Math.max(0, durS);
    if (Math.max(pf, pt) > maxAllowed)
      issues.push({ severity: "error", nodeId: s.id, message: `Target ${Math.round(Math.max(pf, pt))} W exceeds max ${Math.round(maxAllowed)} W` });
    else if (Math.max(pf, pt) > 1.5 * ftp && durS > 60)
      issues.push({ severity: "warning", nodeId: s.id, message: `${Math.round((Math.max(pf, pt) / ftp) * 100)} % FTP for ${Math.round(durS)} s is beyond sprint range` });
    // advance expected HR trace across this block
    const hrEnd = hrSteady((pf + pt) / 2, athlete, phys);
    const tau = hrEnd > hrExp ? 35 : usualTau;
    hrExp = hrEnd + (hrExp - hrEnd) * Math.exp(-durS / tau);

    pf = Math.round(pf * 10) / 10;
    pt = Math.round(pt * 10) / 10;
    const b: FlatBlock = {
      path: r.path,
      iteration: r.iteration,
      nodeId: s.id,
      startS: t,
      durS,
      powerFrom: pf,
      powerTo: pt,
      phase: phases[i] as Phase,
    };
    if (durRangeS) b.durRangeS = durRangeS;
    blocks.push(b);
    t += durS;
    rangeTotal[0] += durRangeS ? durRangeS[0] : durS;
    rangeTotal[1] += durRangeS ? durRangeS[1] : durS;
  });

  // 1 s expected power trace → NP / IF / TSS / zones / W'bal
  const trace: number[] = [];
  const owner: number[] = [];
  blocks.forEach((b, bi) => {
    const n = Math.round(b.durS);
    for (let k = 0; k < n; k++) {
      trace.push(b.powerFrom + (b.powerTo - b.powerFrom) * ((k + 0.5) / Math.max(1, n)));
      owner.push(bi);
    }
  });
  const { np } = normalizedPower(trace);
  const IF = ftp > 0 ? np / ftp : 0;
  const tss = ftp > 0 ? ((trace.length * np * IF) / (ftp * 3600)) * 100 : 0;
  const zones = powerZones(ftp);
  const tiz = new Array<number>(zones.length).fill(0);
  for (const p of trace) tiz[zoneIndex(zones, p)] = (tiz[zoneIndex(zones, p)] as number) + 1;

  if (trace.length) {
    const wb = wbalSeries(trace, 1, phys.cp, phys.wPrime);
    let mi = 0;
    for (let i = 1; i < wb.length; i++) if ((wb[i] as number) < (wb[mi] as number)) mi = i;
    const minW = wb[mi] as number;
    const nodeId = blocks[owner[mi] as number]?.nodeId;
    const pct = Math.round((minW / phys.wPrime) * 100);
    if (minW < 0) issues.push({ severity: "error", nodeId, message: `W'bal drops to ${pct} % — not sustainable as prescribed` });
    else if (minW < 0.15 * phys.wPrime) issues.push({ severity: "warning", nodeId, message: `W'bal drops to ${pct} % — very hard, little reserve` });
  }

  if (w.nodes.length === 0) issues.push({ severity: "error", message: "Workout is empty" });
  else if (t > 20 * 60) {
    const lastB = blocks[blocks.length - 1];
    let coolS = 0;
    for (let i = blocks.length - 1; i >= 0 && (blocks[i] as FlatBlock).phase === "cooldown"; i--) coolS += (blocks[i] as FlatBlock).durS;
    if (!lastB || coolS < 180) issues.push({ severity: "warning", message: "No cool-down (≥ 3 min easy) at the end" });
  }

  return {
    blocks,
    totalS: t,
    totalRangeS: rangeTotal,
    tss: Math.round(tss * 10) / 10,
    if: Math.round(IF * 1000) / 1000,
    timeInZoneS: tiz,
    issues,
  };
}

/** NP from a 1 Hz power trace (30 s rolling mean, 4th-power mean). */
export function normalizedPower(p: readonly number[]): { np: number } {
  if (p.length === 0) return { np: 0 };
  if (p.length < 30) {
    let s = 0;
    for (const x of p) s += x;
    return { np: s / p.length };
  }
  let roll = 0;
  let s4 = 0;
  let n = 0;
  for (let i = 0; i < p.length; i++) {
    roll += p[i] as number;
    if (i >= 30) roll -= p[i - 30] as number;
    if (i >= 29) {
      s4 += (roll / 30) ** 4;
      n++;
    }
  }
  return { np: Math.pow(s4 / n, 0.25) };
}
