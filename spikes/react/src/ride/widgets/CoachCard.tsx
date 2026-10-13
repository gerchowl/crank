import { backend } from "../../backend";
import { useNudges } from "../../lib/live";
import { mmss } from "../../lib/format";
import { WidgetFrame } from "./common";

const VERDICT: Record<string, string> = { ease_off: "Ease off", hold: "Hold", push: "Push", replan: "Replan" };
const BAND_ICON: Record<string, string> = { strong: "●", lean: "◐", uncertain: "○" };

// coach_card — the s1 nudge with its band; accept/decline (Y / N).
export function CoachCard() {
  const nudges = useNudges();
  const pending = nudges.filter((n) => n.status === "pending");
  const n = pending[pending.length - 1] ?? nudges[nudges.length - 1];
  return (
    <WidgetFrame title="Coach" aside={n ? <span className="muted small">{mmss(n.t)}</span> : null} className="coach">
      {!n ? (
        <p className="muted">Watching. Nudges appear here.</p>
      ) : (
        <div className={`coach-body ${n.status}`}>
          <div className="coach-verdict">
            <span className={`band band-${n.band}`}>
              {BAND_ICON[n.band]} {n.band}
            </span>
            <strong>{VERDICT[n.verdict]}</strong>
            {n.proposedPctChange !== undefined && <span className="muted">{n.proposedPctChange > 0 ? "+" : ""}{n.proposedPctChange}%</span>}
          </div>
          <p className="coach-msg">{n.message}</p>
          {n.status === "pending" ? (
            <div className="coach-actions">
              <button type="button" className="btn primary big" onClick={() => { backend.coach.respond(n.callId, true); }}>
                <kbd>Y</kbd> Accept
              </button>
              <button type="button" className="btn big" onClick={() => { backend.coach.respond(n.callId, false); }}>
                <kbd>N</kbd> Decline
              </button>
            </div>
          ) : (
            <p className="coach-status">{n.status.replace("_", " ")}</p>
          )}
        </div>
      )}
    </WidgetFrame>
  );
}
