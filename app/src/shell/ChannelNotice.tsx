// The live data has stopped (ADR-0001 §4.3.2, §4.3.4; DESIGN P6 "a value whose input is lost is never shown as live"):
// the snapshot channel's watchdog found no frame for 2 s, or a frame broke the series rule (a core bug, `internal`).
// Every place but the ride screen shows it here, at the top of the main region in the page's flow, as the ride-load
// notice is (App.tsx `Destination`); the ride screen shows it in its band (the engine-unresponsive banner). It says
// what the page is doing about it, the channel's reopen, and goes when the next frame arrives. ⚠ and a word carry its
// state (DESIGN §18.7 rule 11).
import { useUi } from "../stream/uiHooks";

export function ChannelNotice() {
  const ui = useUi();
  if (!ui.stale) return null;
  const fault = ui.error?.code === "internal" ? ui.error.message : null;
  return (
    <div className="ride-notice-band">
      <p role="alert" className="ride-notice" data-state="error">
        <span className="ride-notice-text" data-testid="channel-stale">
          <span data-state-label>⚠</span> Live data stopped: what is shown may
          be out of date. Reconnecting…
          {fault !== null && ` The core sent a broken update (${fault}).`}
        </span>
      </p>
    </div>
  );
}
