// Settings (DESIGN §5.9). The ride-screen section edits a draft (#8, #11): controls read the
// draft only, Save validates and persists, Cancel/Reset never touch the saved value until Save.
import { useState } from "react";
import { athleteModel, devices } from "../mock/ext";
import { DEFAULT_RIDE_SCREEN, persistNotices, rideScreen, saveRideScreen, setTheme, SLOT_METRICS, theme, type RideScreenConfig, type SlotMetric } from "../persist";
import { navigate, useRoute } from "../session";
import { useStore } from "../store";

const SECTIONS = [
  ["devices", "Devices"],
  ["athlete", "Athlete model"],
  ["coach", "Coach"],
  ["ride-screen", "Ride screen"],
  ["appearance", "Appearance"],
] as const;

const METRIC_NAME: Record<SlotMetric, string> = { hr: "Heart rate", cadence: "Cadence", wbal_pct: "W′ balance", dfa_a1: "DFA α1", hr_vs_typical: "HR vs usual", power_10s: "Power 10 s", kj: "Work (kJ)" };

function RideScreenForm() {
  const saved = useStore(rideScreen);
  const [draft, setDraft] = useState<RideScreenConfig>(saved);
  const [error, setError] = useState<string | null>(null);
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);
  const slot = (key: "slotA" | "slotB", label: string) => (
    <label className="field">
      <span>{label}</span>
      <select value={draft[key]} onChange={(e) => setDraft({ ...draft, [key]: e.target.value as SlotMetric })}>
        {SLOT_METRICS.map((m) => (
          <option key={m} value={m}>
            {METRIC_NAME[m]}
          </option>
        ))}
      </select>
    </label>
  );
  return (
    <>
      <h2>Ride screen</h2>
      <p className="muted">The anatomy is fixed (hero · side · coach · road); you choose what the side slots show. Changes apply after Save.</p>
      <div className="ride-screen-edit">
        <div className="schematic" aria-hidden data-density={draft.density}>
          <div className="s-bar" />
          <div className="s-hero">hero (by phase)</div>
          <div className="s-a">{METRIC_NAME[draft.slotA]}</div>
          <div className="s-b">{METRIC_NAME[draft.slotB]}</div>
          <div className="s-coach">coach</div>
          <div className="s-road">road ahead</div>
        </div>
        <div>
          <h3>Density</h3>
          <div className="choices" role="radiogroup" aria-label="Density">
            {(["focus", "standard", "detail"] as const).map((d) => (
              <button key={d} role="radio" aria-checked={draft.density === d} className={draft.density === d ? "tap wide on" : "tap wide"} onClick={() => setDraft({ ...draft, density: d })}>
                {d}
              </button>
            ))}
          </div>
          {slot("slotA", "Side slot 1")}
          {slot("slotB", "Side slot 2")}
          {draft.slotA === draft.slotB && <p className="status-serious small">⚠ The two slots must show different metrics.</p>}
          {error && <p className="status-serious small">⚠ {error}</p>}
          <div className="actions">
            <button className="primary" disabled={!dirty || draft.slotA === draft.slotB} onClick={() => setError(saveRideScreen(draft))}>
              Save
            </button>
            <button disabled={!dirty} onClick={() => (setDraft(saved), setError(null))}>
              Cancel
            </button>
            <button onClick={() => setDraft(DEFAULT_RIDE_SCREEN)}>Reset to default</button>
          </div>
        </div>
      </div>
    </>
  );
}

export function Settings() {
  const r = useRoute();
  const sec = SECTIONS.find(([id]) => id === r.sub)?.[0] ?? "devices";
  const mode = useStore(theme);
  const notices = useStore(persistNotices);
  const [autonomy, setAutonomy] = useState("auto-apply strong calls");
  return (
    <div className="page settings">
      <nav className="subnav" aria-label="Settings sections">
        {SECTIONS.map(([id, label]) => (
          <button key={id} className={sec === id ? "on" : ""} onClick={() => navigate("settings", id)}>
            {label}
          </button>
        ))}
      </nav>
      <section className="card settings-card">
        {notices.map((n) => (
          <p key={n} className="status-warning small">
            ⚠ {n}
          </p>
        ))}
        {sec === "devices" && (
          <>
            <h2>Devices</h2>
            <p className="muted">One trainer and one HR strap per ride. The NEO allows a single Bluetooth connection: other apps must be closed.</p>
            <ul className="devices">
              {devices.map((d) => (
                <li key={d.id} className={`device ${d.state}`}>
                  <span className="role">{d.role}</span>
                  <b>{d.name}</b>
                  <span className="muted protocol">{d.protocol}</span>
                  <span className={d.state === "connected" ? "status-good" : "muted"}>
                    {d.state === "connected" ? "● connected" : "○ available"}
                    {d.battery !== null && ` · battery ${d.battery} %`}
                  </span>
                  <button>{d.state === "connected" ? "Forget" : "Use this"}</button>
                </li>
              ))}
            </ul>
            <button>Scan for devices</button>
          </>
        )}
        {sec === "athlete" && (
          <>
            <h2>Athlete model</h2>
            <table>
              <tbody>
                {athleteModel.map((e) => (
                  <tr key={e.label}>
                    <td>{e.label}</td>
                    <td className="num">
                      <b>{e.value}</b> ± {e.sd} {e.unit}
                    </td>
                    <td className="muted">
                      {e.source} · {e.updated}
                    </td>
                    <td>
                      <button>Override</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </>
        )}
        {sec === "coach" && (
          <>
            <h2>Coach autonomy</h2>
            <div className="choices" role="radiogroup" aria-label="Autonomy">
              {["auto-apply strong calls", "always ask", "off (log only)"].map((o) => (
                <button key={o} role="radio" aria-checked={autonomy === o} className={autonomy === o ? "tap wide on" : "tap wide"} onClick={() => setAutonomy(o)}>
                  {o}
                </button>
              ))}
            </div>
            <p className="muted small">Strong calls auto-apply with a 15 s undo (U). Unanswered questions keep the plan. Persisted via #29.</p>
          </>
        )}
        {sec === "ride-screen" && <RideScreenForm />}
        {sec === "appearance" && (
          <>
            <h2>Appearance</h2>
            <div className="choices" role="radiogroup" aria-label="Mode">
              {(["dark", "light"] as const).map((m) => (
                <button key={m} role="radio" aria-checked={mode === m} className={mode === m ? "tap wide on" : "tap wide"} onClick={() => setTheme(m)}>
                  {m}
                </button>
              ))}
            </div>
            <p className="muted small">Ocean preset. Auto mode, other presets and classic zones per spec/THEMES.md; palette validation #30.</p>
          </>
        )}
      </section>
    </div>
  );
}
