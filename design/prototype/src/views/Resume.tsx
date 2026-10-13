// Resume sheet (DESIGN §6.6): shown first on launch when a ride was interrupted by a crash,
// forced quit or sleep. The record survives up to its last 5 s save (ADR-0001 §6).
import { demoWorkout } from "@crank/shared";
import { mmss } from "../format";
import { interruptedRide } from "../mock/ext";
import { backend, loadRide, navigate } from "../session";

export function Resume() {
  const r = interruptedRide;
  return (
    <div className="page resume">
      <section className="card ready-card" role="alertdialog" aria-label="Unfinished ride">
        <span className="eyebrow">Unfinished ride</span>
        <h1>{r.workout}</h1>
        <p className="big-meta">
          Stopped {r.stoppedAt} · {mmss(r.riddenS)} ridden · {r.block}
        </p>
        <ul className="checks">
          <li className="status-good">
            <span aria-hidden>✓</span>
            <span>
              <b>{mmss(r.savedS)} saved</b> — {r.gapNote}, so at most the last {mmss(r.riddenS - r.savedS)} are missing
            </span>
          </li>
          <li className="muted">
            <span aria-hidden>i</span>
            <span>Resuming continues the same ride from Threshold rep 1; the time in between is recorded as no data and ERG eases back in over 5 s.</span>
          </li>
        </ul>
        <div className="actions">
          <button className="primary big" autoFocus onClick={() => (loadRide(demoWorkout(backend.athlete.ftp)), navigate("ready"))}>
            Resume ride
          </button>
          <button className="big" onClick={() => navigate("history")}>
            Save & review
          </button>
          <span className="spacer" />
          <button onClick={() => navigate("today")}>Discard…</button>
        </div>
      </section>
    </div>
  );
}
