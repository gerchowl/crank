// Resume sheet (DESIGN §6.6): shown first on launch when a ride was interrupted by a crash,
// forced quit or sleep. The record survives up to its last 5 s save (ADR-0001 §6). How the rider
// re-enters depends on how long ago the ride stopped and where (RESUME policy in mock/ext.ts, #50).
import { demoWorkout } from "@crank/shared";
import { mmss } from "../format";
import { devices, interruptedRide, resumePlan, RESUME, sceneFixture } from "../mock/ext";
import { backend, navigate } from "../session";
import { requestStart } from "../safety";
import { useStore } from "../store";
import { KeyHint } from "./Ride";

const ago = (s: number) => (s < 3600 ? `${Math.round(s / 60)} min ago` : s < 86400 ? `${Math.round(s / 3600)} h ago` : "yesterday");

export function Resume() {
  const fx = useStore(sceneFixture);
  const r = interruptedRide;
  const agoS = fx.stoppedAgoS ?? 240;
  const plan = resumePlan(agoS, r.inWork);
  const trainerOk = fx.status?.trainer !== "lost";
  const resume = () => requestStart(demoWorkout(backend.athlete.ftp), "resume");
  return (
    <div className="page resume saddle-screen">
      <section className="card ready-card" role="alertdialog" aria-label="Unfinished ride">
        <span className="eyebrow">Unfinished ride · stopped {ago(agoS)}</span>
        <h1>{r.workout}</h1>
        <p className="big-meta">
          {mmss(r.riddenS)} ridden · {r.block}
        </p>
        <ul className="checks">
          <li className="status-good">
            <span aria-hidden>✓</span>
            <span>
              <b>{mmss(r.savedS)} saved</b> — {r.gapNote}, so at most the last {mmss(r.riddenS - r.savedS)} are missing
            </span>
          </li>
          {plan.kind !== "stale" && (
            <li className={trainerOk ? "status-good" : "status-warning"}>
              <span aria-hidden>{trainerOk ? "✓" : "…"}</span>
              <span>
                <b>{devices[0]?.name}</b> {trainerOk ? "connected" : "not connected yet — resuming waits for it"}
              </span>
            </li>
          )}
          <li className="muted">
            <span aria-hidden>i</span>
            <span>
              {plan.kind === "stale"
                ? `Stopped more than ${RESUME.rewarmS / 60} min ago: picking up a threshold rep cold is not the same session. Save what you rode; start fresh with a warm-up if you want to ride the rest.`
                : `Resume ${plan.note}. The time in between is recorded as no data.`}
            </span>
          </li>
        </ul>
        <div className="actions saddle">
          {plan.kind === "stale" ? (
            <>
              <button className="primary" autoFocus onClick={() => navigate("history")}>
                <KeyHint a="coach" /> Save & review
              </button>
              <button onClick={resume}>Ride the rest as a new ride</button>
            </>
          ) : (
            <>
              <button className="primary" autoFocus disabled={!trainerOk} onClick={resume}>
                <KeyHint a="coach" /> {plan.kind === "rewarm" ? "Resume with re-warm-up" : "Resume ride"}
              </button>
              <button onClick={() => navigate("history")}>Save & review</button>
            </>
          )}
          <span className="spacer" />
          <button onClick={() => navigate("today")}>Discard…</button>
        </div>
      </section>
    </div>
  );
}
