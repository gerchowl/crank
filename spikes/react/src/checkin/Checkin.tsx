import { useActionState } from "react";
import type { Checkin as CheckinT, Proposal } from "@neo/shared";
import { backend } from "../backend";
import { go } from "../lib/router";
import { editorSeed } from "../lib/session";
import { forceRefreshCurrent } from "../lib/live";
import { mmss } from "../lib/format";
import { MiniProfile } from "../components/MiniProfile";

type Score = 1 | 2 | 3 | 4 | 5;
const ROWS: { key: "sleep" | "legs" | "stress" | "motivation"; label: string; lo: string; hi: string }[] = [
  { key: "sleep", label: "Sleep", lo: "awful", hi: "great" },
  { key: "legs", label: "Legs", lo: "dead", hi: "fresh" },
  { key: "stress", label: "Stress", lo: "calm", hi: "frazzled" },
  { key: "motivation", label: "Motivation", lo: "none", hi: "fired up" },
];
const MINUTES = [30, 45, 60, 90];

const score = (v: FormDataEntryValue | null): Score => {
  const n = Number(v);
  return n === 1 || n === 2 || n === 3 || n === 4 || n === 5 ? n : 3;
};

interface Result { checkin: CheckinT; proposal: Proposal }

// React 19 form action: FormData in, Promise<Proposal> out; isPending covers the planner latency.
async function propose(_prev: Result | null, fd: FormData): Promise<Result> {
  const custom = Number(fd.get("customMinutes"));
  const minutes = fd.get("minutes") === "custom" && custom > 0 ? custom : Number(fd.get("minutes") ?? 60);
  const rawNote = fd.get("note");
  const note = typeof rawNote === "string" ? rawNote.trim() : "";
  const checkin: CheckinT = {
    sleep: score(fd.get("sleep")), legs: score(fd.get("legs")), stress: score(fd.get("stress")), motivation: score(fd.get("motivation")),
    minutes, ...(note ? { note } : {}),
  };
  return { checkin, proposal: await backend.planner.propose(checkin) };
}

/** V2 check-in (≤ 10 s): 4 × tap 1–5, minutes chips, optional note → proposal → accept / regenerate / edit. */
export function Checkin() {
  const [result, submit, pending] = useActionState(propose, null);
  const p = result?.proposal;
  // React 19 resets uncontrolled forms after an action; defaults follow the last submission so it sticks
  const lastMinutes = result ? (MINUTES.includes(result.checkin.minutes) ? String(result.checkin.minutes) : "custom") : "60";
  const compiled = p ? backend.compileWorkout(p.workout) : null;
  return (
    <div className="checkin">
      <form action={submit} className="ci-form">
        <h2>How are you today?</h2>
        {ROWS.map((r) => (
          <fieldset key={r.key} className="ci-row">
            <legend>{r.label}</legend>
            <span className="muted small">{r.lo}</span>
            <div className="ci-chips">
              {[1, 2, 3, 4, 5].map((v) => (
                <label key={v} className="chip">
                  <input type="radio" name={r.key} value={v} defaultChecked={v === (result?.checkin[r.key] ?? 3)} />
                  <span>{v}</span>
                </label>
              ))}
            </div>
            <span className="muted small">{r.hi}</span>
          </fieldset>
        ))}
        <fieldset className="ci-row">
          <legend>Time</legend>
          <div className="ci-chips">
            {[...MINUTES.map(String), "custom"].map((m) => (
              <label key={m} className="chip wide">
                <input type="radio" name="minutes" value={m} defaultChecked={m === lastMinutes} />
                <span>{m === "custom" ? "custom" : `${m}′`}</span>
              </label>
            ))}
            <input type="number" name="customMinutes" min={15} max={300} defaultValue={result && lastMinutes === "custom" ? result.checkin.minutes : 75} aria-label="custom minutes" className="ci-custom" />
          </div>
        </fieldset>
        <label className="ci-note">
          <span className="muted">How do you feel? (optional)</span>
          <input type="text" name="note" placeholder="e.g. legs heavy after yesterday" defaultValue={result?.checkin.note ?? ""} />
        </label>
        <button type="submit" className="btn primary big" disabled={pending}>
          {pending ? "Planning…" : p ? "↻ Regenerate" : "Get today's workout →"}
        </button>
      </form>

      <section className={`ci-proposal ${pending ? "pending" : ""}`} aria-live="polite">
        {!p || !compiled ? (
          <p className="muted">{pending ? "The planner is thinking…" : "Your proposal appears here."}</p>
        ) : (
          <>
            <div className="readiness">
              {ROWS.map((r) => {
                const v = result.checkin[r.key];
                const good = r.key === "stress" ? v <= 2 : v >= 4;
                const bad = r.key === "stress" ? v >= 4 : v <= 2;
                return (
                  <div key={r.key} className="rtile">
                    <span className="label">{r.label}</span>
                    <b>{"●".repeat(v)}<span className="muted">{"○".repeat(5 - v)}</span></b>
                    <span className={`status-tag ${good ? "good" : bad ? "warning" : ""}`}>{good ? "✓ good" : bad ? "! low" : "– ok"}</span>
                  </div>
                );
              })}
            </div>
            <h3>{p.workout.name}</h3>
            <p className="muted">{mmss(compiled.totalS)} · TSS {Math.round(compiled.tss)} · IF {compiled.if.toFixed(2)}</p>
            <MiniProfile blocks={compiled.blocks} height={110} label="Proposed workout profile" />
            <p className="rationale"><b>Why:</b> {p.rationale}</p>
            <div className="row">
              <button
                type="button"
                className="btn primary big"
                onClick={() => {
                  backend.ride.load(p.workout);
                  backend.ride.start();
                  forceRefreshCurrent();
                  go("ride");
                }}
              >
                ✓ Accept & ride
              </button>
              <button type="button" className="btn big" onClick={() => { editorSeed.set(p.workout); go("editor"); }}>✎ Edit</button>
            </div>
          </>
        )}
      </section>
    </div>
  );
}
