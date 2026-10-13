import { Match, Show, Switch } from "solid-js";
import type { CoachNudge } from "@neo/shared";
import { clock } from "../lib/format";
import { latestNudge, ride } from "../state/ride";
import { BandChip, StatusBadge, type StatusKind } from "./chips";

const VERDICT: Record<CoachNudge["verdict"], string> = { ease_off: "Ease off", hold: "Hold", push: "Push", replan: "Replan" };
const STATUS: Record<CoachNudge["status"], [StatusKind, string]> = {
  pending: ["warning", "waiting for you"],
  auto_applied: ["good", "auto-applied"],
  accepted: ["good", "accepted"],
  declined: ["serious", "declined"],
  expired: ["serious", "expired"],
};
const TTL: Record<CoachNudge["band"], number> = { strong: 0, lean: 30, uncertain: 20 };

/** Coach card — s1 nudge text + band + accept/decline (keys Y / N). */
export function CoachCard() {
  const nudge = () => latestNudge();
  return (
    <div class="coach">
      <div class="w-label">
        <span>Coach</span>
        <span class="muted">{ride.nudges.length} decisions</span>
      </div>
      <Show when={nudge()} fallback={<div class="empty">Coach is watching — nothing to say yet.</div>}>
        {(n) => (
          <div class="coach-body" classList={{ pending: n().status === "pending" }}>
            <div class="coach-head">
              <span class="coach-verdict">
                {VERDICT[n().verdict]}
                <Show when={n().proposedPctChange}>
                  {(p) => (
                    <span class="muted">
                      {" "}
                      {p() > 0 ? "+" : ""}
                      {p()}%
                    </span>
                  )}
                </Show>
              </span>
              <BandChip band={n().band} p={n().p} />
            </div>
            <p class="coach-msg">{n().message}</p>
            <Switch>
              <Match when={n().status === "pending"}>
                <div class="coach-actions">
                  <button class="big good" onClick={() => ride.controls.respond(n().callId, true)}>
                    <kbd>Y</kbd> Accept
                  </button>
                  <button class="big" onClick={() => ride.controls.respond(n().callId, false)}>
                    <kbd>N</kbd> Decline
                  </button>
                  <span class="muted">expires in {clock(Math.max(0, n().t + TTL[n().band] - ride.status.t))}</span>
                </div>
              </Match>
              <Match when={n().status !== "pending"}>
                <div class="coach-actions">
                  <StatusBadge kind={STATUS[n().status][0]} label={STATUS[n().status][1]} />
                  <span class="muted">at {clock(n().t)}</span>
                </div>
              </Match>
            </Switch>
          </div>
        )}
      </Show>
    </div>
  );
}
