// The athlete model as the app reads it (ADR-0009 §2.1.3, §2.1.4): one copy of `athlete_model()` for every view that
// shows it, read again after each change (the commands answer nothing, ADR-0001 §4.2: the model is durable data, not
// a channel). Settings → Athlete model shows and edits it; `useAthleteHr` reads its heart-rate references.
//
// The live ride does not read it: its zones come from the loaded ride's frozen snapshot (`RidePlan.athlete`, through
// `snapshotHrRefs`), so a Settings change mid-ride does not move the live zone (§2.1.4).
import { useEffect, useMemo, useSyncExternalStore } from "react";
import { api } from "./api";
import {
  commands,
  type ApiError,
  type AthleteModel,
  type AthleteParam,
  type AthleteSnapshotView,
  type ResolvedView,
} from "./bindings";
import type { HrRefs } from "./ride/hrZone";

/** The model as last read, and the last read's refusal. */
export interface AthleteRead {
  /** `null` until the first read has answered. */
  model: AthleteModel | null;
  /** The last read's refusal; a later good read clears it. */
  error: ApiError | null;
}

const NONE: AthleteRead = { model: null, error: null };
let read: AthleteRead = NONE;
let asked = 0;
const listeners = new Set<() => void>();
const publish = (next: AthleteRead) => {
  read = next;
  for (const l of listeners) l();
};

/** Reads the model again; the answer of the last call asked wins. */
export async function refreshAthleteModel(): Promise<void> {
  const call = ++asked;
  const r = await api(commands.athleteModel());
  if (call !== asked) return;
  publish(
    r.status === "ok"
      ? { model: r.data, error: null }
      : { model: read.model, error: r.error },
  );
}

/** Forgets the model read (tests: each starts from a fresh core). */
export function forgetAthleteModel(): void {
  asked++;
  publish(NONE);
}

/** The athlete model, read when the calling view mounts. */
export function useAthleteModel(): AthleteRead {
  useEffect(() => {
    void refreshAthleteModel();
  }, []);
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => read,
  );
}

/** A value that is a zone reference: measured or the rider's own, never a prior (ADR-0009 §2.1.2). */
const measured = (r: ResolvedView | null | undefined): number | null =>
  r && r.source !== "prior" ? r.value : null;

/** A parameter's resolved value in the model, `null` while unset or unread. */
export function resolvedOf(
  model: AthleteModel | null,
  param: AthleteParam,
): ResolvedView | null {
  return model?.params.find((p) => p.param === param)?.resolved ?? null;
}

/** The HR references of a ride's snapshot (crank-domain snapshot.rs `AthleteSnapshot::hr_refs`). */
export function snapshotHrRefs(s: AthleteSnapshotView | null): HrRefs {
  return { lthrBpm: measured(s?.lthr), hrMaxBpm: measured(s?.hrMax) };
}

/** The athlete's LTHR and HR max now, bpm, from the model; each `null` while unknown, a prior, or unread. */
export function useAthleteHr(): HrRefs {
  const { model } = useAthleteModel();
  return useMemo(
    () => ({
      lthrBpm: measured(resolvedOf(model, "lthr")),
      hrMaxBpm: measured(resolvedOf(model, "hr_max")),
    }),
    [model],
  );
}
