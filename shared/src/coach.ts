import type { Band, CoachNudge, HrRecoveryState, StepNode } from "./contract";
import { Rng } from "./rng";
import type { CoachFeatures } from "./sim";

export function bandOf(p: number): Band {
  return p >= 0.85 ? "strong" : p >= 0.65 ? "lean" : "uncertain";
}

const sigmoid = (x: number) => 1 / (1 + Math.exp(-x));

export interface CoachHost {
  features(): CoachFeatures;
  usualTauS: number;
  /** apply a % change to the remaining work blocks */
  apply(pct: number): void;
  /** end the current recovery block if it is still `stepId` */
  endRecovery(stepId: string): void;
  emit(n: CoachNudge): void;
}

interface Meta {
  expiresT: number;
  endRecoveryOf?: string;
  rating?: { rating: "right" | "wrong" | "unsure"; useful: boolean };
}

/** Lifetime of a pending nudge (ride seconds) by band. */
export const NUDGE_TTL_S: Record<Band, number> = { strong: 0, lean: 30, uncertain: 20 };
/** Minimum ride time between two rule-based nudges. */
export const NUDGE_COOLDOWN_S = 120;

/**
 * Mock System-1 coach: simple rules over simulated features produce a probability p; the band
 * follows from p. Strong nudges are auto-applied, lean/uncertain ones wait for respond() and
 * expire after NUDGE_TTL_S.
 */
export class Coach {
  private nudges: CoachNudge[] = [];
  private meta = new Map<string, Meta>();
  private lastT = -Infinity;
  private seq = 0;
  private rng: Rng;

  constructor(private host: CoachHost, private seed: number) {
    this.rng = new Rng(seed ^ 0xc0ac4);
  }

  reset(): void {
    this.nudges = [];
    this.meta.clear();
    this.lastT = -Infinity;
    this.seq = 0;
    this.rng = new Rng(this.seed ^ 0xc0ac4);
  }

  decisions(): CoachNudge[] {
    return this.nudges.map((n) => ({ ...n }));
  }

  ratingOf(callId: string): Meta["rating"] {
    return this.meta.get(callId)?.rating;
  }

  private pending(): CoachNudge | undefined {
    return this.nudges.find((n) => n.status === "pending");
  }

  private create(
    t: number,
    verdict: CoachNudge["verdict"],
    p: number,
    msg: string | ((b: Band) => string),
    pctChange: number | undefined | ((b: Band) => number),
    extra: Partial<Meta> = {},
  ): CoachNudge {
    const pp = Math.min(0.99, Math.max(0.5, p + this.rng.normal(0, 0.02)));
    const band = bandOf(pp);
    const message = typeof msg === "function" ? msg(band) : msg;
    const pct = typeof pctChange === "function" ? pctChange(band) : pctChange;
    this.seq++;
    const n: CoachNudge = {
      callId: `s1-${this.seed.toString(36)}-${this.seq}`,
      t,
      verdict,
      band,
      p: Math.round(pp * 100) / 100,
      message,
      status: "pending",
    };
    if (pct !== undefined) n.proposedPctChange = pct;
    this.meta.set(n.callId, { expiresT: t + NUDGE_TTL_S[band], ...extra });
    this.nudges.push(n);
    this.lastT = t;
    if (band === "strong") {
      n.status = "auto_applied";
      this.execute(n);
    }
    this.host.emit({ ...n });
    return n;
  }

  private execute(n: CoachNudge): void {
    const m = this.meta.get(n.callId);
    if (m?.endRecoveryOf) this.host.endRecovery(m.endRecoveryOf);
    else if (n.proposedPctChange) this.host.apply(n.proposedPctChange);
  }

  private setStatus(n: CoachNudge, s: CoachNudge["status"]): void {
    n.status = s;
    this.host.emit({ ...n });
  }

  expire(t: number): void {
    for (const n of this.nudges) {
      if (n.status === "pending" && t >= (this.meta.get(n.callId)?.expiresT ?? 0)) this.setStatus(n, "expired");
    }
  }

  respond(callId: string, accept: boolean): void {
    const n = this.nudges.find((x) => x.callId === callId);
    if (!n || n.status !== "pending") return;
    if (accept) {
      n.status = "accepted";
      this.execute(n);
      this.host.emit({ ...n });
    } else this.setStatus(n, "declined");
  }

  rate(callId: string, rating: "right" | "wrong" | "unsure", useful: boolean): void {
    const m = this.meta.get(callId);
    if (m) m.rating = { rating, useful };
  }

  /** Periodic rule evaluation (every 10 s of ride time). */
  evaluate(t: number): void {
    this.expire(t);
    if (t < 300 || this.pending() || t - this.lastT < NUDGE_COOLDOWN_S) return;
    const f = this.host.features();
    if (f.phase !== "work" || f.blockElapsed < 15) return;
    const hv = f.hrVsTypical ?? 0;
    const dec = f.decoupling ?? 0;

    const ease = 0.55 * (hv - 5.5) + 0.12 * (Math.max(0, dec) - 5) + (f.wbalPct < 12 ? 0.15 * (12 - f.wbalPct) : 0);
    const pe = sigmoid(ease);
    if (pe >= 0.6) {
      this.create(
        t,
        "ease_off",
        pe,
        (band) =>
          `HR is ${hv.toFixed(0)} bpm above your usual for this power${f.wbalPct < 25 ? ` and W'bal is at ${f.wbalPct.toFixed(0)} %` : ""}. Ease off ${band === "strong" ? 5 : 3} % on the remaining work.`,
        (band) => (band === "strong" ? -5 : -3),
      );
      return;
    }

    if (hv < 0 && f.wbalPct > 55 && (f.compliance ?? 0) >= 80) {
      const pp = sigmoid(0.6 * (-hv - 1) + 0.03 * (f.wbalPct - 55));
      if (pp >= 0.6) {
        this.create(t, "push", pp, `HR ${(-hv).toFixed(0)} bpm below usual, W'bal ${f.wbalPct.toFixed(0)} % and you're on target. Try +3 %.`, 3);
        return;
      }
    }

    if (f.cadence10s !== null && f.cadence10s < 82 && f.power3s !== null && f.power3s >= 0.9 * f.target) {
      const ph = sigmoid(0.35 * (84 - f.cadence10s));
      if (ph >= 0.55) this.create(t, "hold", Math.max(0.55, ph), `Cadence fading (${f.cadence10s.toFixed(0)} rpm). Hold the target, spin a little lighter.`, undefined);
    }
  }

  /** After an hr_recovery block: suggest replanning when recovery is slowing. */
  onRecoveryEnded(st: HrRecoveryState, t: number): void {
    if (!st.fit) return;
    this.expire(t);
    if (this.pending()) return;
    const ratio = st.fit.tauS / st.fit.usualTauS;
    if (ratio < 1.25 && !st.unreachable) return;
    const p = sigmoid(6 * (ratio - 1.25) + (st.unreachable ? 1.5 : 0) + 0.8);
    this.create(
      t,
      "replan",
      p,
      `Recovery is slowing: τ ${st.fit.tauS.toFixed(0)} s vs usual ${st.fit.usualTauS.toFixed(0)} s${st.unreachable ? `, HR levelled off near ${st.fit.hrInf.toFixed(0)} bpm` : ""}. Suggest −5 % on the remaining intervals.`,
      -5,
    );
  }

  /** hr_recovery with onUnreachable = ask_coach: ask whether to end the recovery now. */
  onUnreachable(st: HrRecoveryState, step: StepNode, t: number): void {
    this.expire(t);
    const hrInf = st.fit?.hrInf ?? st.targetBpm;
    this.create(
      t,
      "replan",
      0.75,
      `HR is levelling off near ${hrInf.toFixed(0)} bpm — the ${st.targetBpm} bpm target looks unreachable. End the recovery now and continue?`,
      undefined,
      { endRecoveryOf: step.id },
    );
  }
}
