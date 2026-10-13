import { backend } from "../backend";
import { useMetric, useRideStatus } from "../lib/live";
import { mmss } from "../lib/format";
import { ThemeMenu } from "../theme/ThemeMenu";

function Clock() {
  const elapsed = useMetric("elapsed", 2)?.v ?? null;
  const remaining = useMetric("remaining", 1)?.v ?? null;
  return (
    <span className="clock" title="elapsed / remaining">
      <b>{mmss(elapsed)}</b>
      <span className="muted"> / −{mmss(remaining)}</span>
    </span>
  );
}

const DOT: Record<string, string> = { connected: "●", connecting: "◌", lost: "✕" };
function Conn({ label, state }: { label: string; state: string }) {
  const tone = state === "connected" ? "good" : state === "connecting" ? "warning" : "critical";
  return (
    <span className={`conn status-${tone}`} title={`${label}: ${state}`}>
      <span className="conn-dot">{DOT[state]}</span> {label}
      {state !== "connected" && <span className="small"> {state}</span>}
    </span>
  );
}

export function TopBar(props: { editing: boolean; onEdit: () => void; onHelp: () => void; onExit: () => void }) {
  const state = useRideStatus((s) => s?.state ?? "idle");
  const mode = useRideStatus((s) => s?.mode ?? "erg");
  const trainer = useRideStatus((s) => s?.trainer ?? "connecting");
  const hr = useRideStatus((s) => s?.hrSensor ?? "connecting");
  const offset = useRideStatus((s) => s?.manualOffsetPct ?? 0);
  const speed = useRideStatus((s) => s?.speed ?? 1);
  const phase = useRideStatus((s) => s?.phase ?? "free");
  const riding = state === "riding";

  return (
    <header className="topbar" data-phase={phase} data-state={state}>
      <Clock />
      <span className="pill">{mode.toUpperCase()}</span>
      <span className="pill muted">{phase}</span>
      <Conn label="NEO" state={trainer} />
      <Conn label="HR" state={hr} />
      <span className="spacer" />
      <div className="btn-group" aria-label="manual intensity">
        <button type="button" className="btn big" onClick={() => { backend.ride.adjust(-5); }} title="−5 % (key −)">−5%</button>
        <span className="offset">{offset > 0 ? "+" : ""}{offset}%</span>
        <button type="button" className="btn big" onClick={() => { backend.ride.adjust(5); }} title="+5 % (key +)">+5%</button>
      </div>
      <button type="button" className="btn big" onClick={() => { backend.ride.skipBlock(); }} title="Skip block (S)">Skip ⏭</button>
      <button
        type="button"
        className="btn big primary"
        title="Pause / resume (space)"
        onClick={() => {
          if (riding) backend.ride.pause();
          else if (state === "paused") backend.ride.resume();
          else if (state === "idle") backend.ride.start();
        }}
      >
        {riding ? "⏸ Pause" : state === "finished" ? "Done" : "▶ Ride"}
      </button>
      <div className="seg" aria-label="simulation speed">
        {([1, 10, 60] as const).map((s) => (
          <button type="button" key={s} className={s === speed ? "on" : ""} onClick={() => { backend.ride.setSpeed(s); }}>
            {s}×
          </button>
        ))}
      </div>
      <button type="button" className={`btn ${props.editing ? "primary" : ""}`} onClick={props.onEdit} title="Edit layout (E)">
        ✎ Layout
      </button>
      <ThemeMenu />
      <button type="button" className="btn" onClick={props.onHelp} title="Shortcuts (?)">?</button>
      <button type="button" className="btn" onClick={props.onExit} title="End ride → review">Exit</button>
    </header>
  );
}
