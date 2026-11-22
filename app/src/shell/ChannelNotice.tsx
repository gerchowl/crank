// The live data has stopped or never started (ADR-0001 §4.3.2, §4.3.4; DESIGN P6 "a value whose input is lost is never
// shown as live"):
// - stale: the snapshot channel's watchdog found no frame for 2 s, or a frame broke the series rule (a core bug,
//   `internal`, whose text it adds);
// - refused: `ui_open` was refused (`storage`: the pairings cannot be read; `conflict`: 16 channels open) and no frame
//   has come; the watchdog asks again every 2 s, Retry at once.
// Every place but the ride screen shows it here, at the top of the main region in the page's flow, as the ride-load
// notice is (App.tsx `Destination`); the ride screen shows it itself (its band, or its empty page). A refusal is not
// repeated where the view already shows it beside the devices it hides (Today, Settings → Devices). It goes with the
// next frame. ⚠ and a word carry its state (DESIGN §18.7 rule 11), and its button is the 32 px family (§18.4).
import { useRoute } from "../router";
import { useUi, useUiClient } from "../stream/uiHooks";

export function ChannelNotice() {
  const ui = useUi();
  const client = useUiClient();
  const route = useRoute();
  const refused = !ui.received ? ui.error : null;
  const shownByView =
    route.view === "today" ||
    (route.view === "settings" && route.sub === "devices");
  if (refused && !shownByView)
    return (
      <div className="ride-notice-band">
        <p role="alert" className="ride-notice" data-state="error">
          <span className="ride-notice-text" data-testid="channel-refused">
            <span data-state-label>⚠</span> Live data is not available:{" "}
            {refused.message}. Trying again every 2 s.
          </span>
          <span className="ride-notice-actions">
            <button type="button" onClick={() => client.reopen()}>
              Retry
            </button>
          </span>
        </p>
      </div>
    );
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
