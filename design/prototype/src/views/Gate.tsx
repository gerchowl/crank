// Start gate (DESIGN §5.8): where requestStart() sends a start it holds — today's red-flag re-check
// if it hasn't been answered, or the medical stop if one is active. Same rule for every start path.
import { useState } from "react";
import { HEALTH_QS, RED_FLAGS, type HealthAnswer, type HealthId } from "../mock/ext";
import { capViolations, pendingStart, recordRedFlagCheck, requestStart, START_PATH_NAME, stopActive } from "../safety";
import { navigate, useRoute } from "../session";
import { useStore } from "../store";

const RED = HEALTH_QS.filter((h) => (RED_FLAGS as readonly string[]).includes(h.id));

/** The daily re-check: every red flag of §5.8, each answered, no default. */
export function RedFlagCheck({ onDone, compact = false }: { onDone: (clear: boolean) => void; compact?: boolean }) {
  const [a, setA] = useState<Partial<Record<HealthId, HealthAnswer>>>({});
  const complete = RED.every((h) => a[h.id] === "yes" || a[h.id] === "no");
  return (
    <div className={compact ? "redflag-check compact" : "redflag-check"} aria-label="Red-flag symptoms since your last ride">
      <p className="rf-title">
        <b>Since your last ride</b>, have you had any of these?
      </p>
      {RED.map((h) => (
        <div key={h.id} className={a[h.id] ? "yn" : "yn unanswered"} role="radiogroup" aria-label={h.q}>
          <span>{h.q}</span>
          <span className="choices">
            {(["no", "yes"] as const).map((v) => (
              <button key={v} role="radio" aria-checked={a[h.id] === v} className={a[h.id] === v ? "tap wide on" : "tap wide"} onClick={() => setA((p) => ({ ...p, [h.id]: v }))}>
                {v}
              </button>
            ))}
          </span>
        </div>
      ))}
      <div className="actions">
        <button className="primary" disabled={!complete} onClick={() => onDone(recordRedFlagCheck(a))}>
          {complete ? "Done" : `Answer all ${RED.length}`}
        </button>
      </div>
    </div>
  );
}

export function Gate() {
  const r = useRoute();
  const pending = useStore(pendingStart);
  const blocked = r.sub === "blocked" || stopActive();
  const from = pending ? START_PATH_NAME[pending.from] : "this screen";
  if (r.sub === "capped" && pending)
    return (
      <div className="page gate">
        <section className="card cap-gate" role="alert" aria-label="Ride blocked by the health cap">
          <span className="eyebrow">Ride not started · from {from}</span>
          <h2>Above your injury cap</h2>
          <p>
            <b>“{pending.workout.name}”</b> has {capViolations(pending.workout).join(", and ")}. While the cap is on, crank only starts ERG rides that stay at or below it, so your power can't pass it.
          </p>
          <div className="actions">
            <button className="primary" autoFocus onClick={() => navigate("today")}>
              Back to Today
            </button>
            <button onClick={() => navigate("settings", "health")}>Health settings</button>
          </div>
        </section>
      </div>
    );
  return (
    <div className="page gate">
      {blocked ? (
        <section className="card medical-stop" role="alert" aria-label="Ride blocked by the medical stop">
          <span className="eyebrow">Ride not started · from {from}</span>
          <h2>Medical stop: no planned or coached ride</h2>
          <p>
            {pending ? <b>“{pending.workout.name}”</b> : "This ride"} can't start while the medical stop is active — the same rule applies on every screen. Please see a doctor before training.{" "}
            <b>If you have chest pain now, stop and call emergency services.</b>
          </p>
          <div className="actions">
            <button className="primary" autoFocus onClick={() => navigate("today")}>
              Back to Today
            </button>
            <button onClick={() => navigate("today", "free")}>Ride without a plan…</button>
            <button onClick={() => navigate("settings", "health")}>Medical clearance</button>
          </div>
        </section>
      ) : (
        <section className="card" aria-label="Red-flag check before starting">
          <span className="eyebrow">Before you start · from {from}</span>
          <h2>Quick health check</h2>
          <RedFlagCheck
            onDone={(clear) => {
              const p = pendingStart.get();
              if (clear && p) requestStart(p.workout, p.from);
              else navigate("gate", "blocked");
            }}
          />
        </section>
      )}
    </div>
  );
}
