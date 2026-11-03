// Settings (DESIGN §5.9). The ride-screen section edits a draft (#8, #11): controls read the
// draft only, Save validates and persists, Cancel/Reset never touch the saved value until Save.
import { useEffect, useState } from "react";
import { athleteModel, devices, sceneFixture, type Clearance } from "../mock/ext";
import {
  ACTIONS,
  autonomy,
  bindings,
  DEFAULT_BINDINGS,
  DEFAULT_RIDE_SCREEN,
  distanceM,
  keyGlyph,
  persistNotices,
  rideScreen,
  saveBindings,
  saveRideScreen,
  saveSound,
  setAutonomy,
  setDistance,
  setTheme,
  SLOT_METRICS,
  sound,
  SOUND_EVENTS,
  theme,
  type Action,
  type Autonomy,
  type RideScreenConfig,
  type SlotMetric,
  type SoundEvent,
} from "../persist";
import { play } from "../sound";
import { navigate, useRoute } from "../session";
import { useStore } from "../store";
import { ProfileForm } from "./Profile";
import { Background } from "./Why";

const SECTIONS = [
  ["devices", "Devices"],
  ["profile", "Training profile"],
  ["athlete", "Athlete model"],
  ["health", "Health"],
  ["coach", "Coach"],
  ["controls", "Controls & sound"],
  ["ride-screen", "Ride screen"],
  ["appearance", "Appearance"],
  ["background", "Background"],
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
          <DistanceField />
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
  const auto = useStore(autonomy);
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
        {sec === "health" && <HealthForm />}
        {sec === "profile" && <ProfileForm />}
        {sec === "background" && <Background />}
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
            <div className="choices column" role="radiogroup" aria-label="Autonomy">
              {AUTONOMY.map(([v, l, d]) => (
                <button key={v} role="radio" aria-checked={auto === v} className={auto === v ? "tap wide on" : "tap wide"} onClick={() => setAutonomy(v)}>
                  <b>{l}</b>
                  <span>{d}</span>
                </button>
              ))}
            </div>
            <h3>Limits (always on)</h3>
            <ul className="small">
              <li>One call changes the remaining work by at most −5 % or +3 %.</li>
              <li>All coach changes together stay within −10 % … +5 % of the plan.</li>
              <li>Increases always ask; silence never raises the load. No push without HR or while the health cap is on.</li>
              <li>Coach only says yes; Keep is the only way back to the plan; presses within 1.5 s of a new call are ignored.</li>
            </ul>
          </>
        )}
        {sec === "controls" && <ControlsForm />}
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

const AUTONOMY: [Autonomy, string, string][] = [
  ["ask", "Always ask (default)", "Every call asks; no answer keeps the plan."],
  ["auto_decreases", "Auto-apply strong decreases", "Strong ease-off calls apply at once with a 15 s undo; increases still ask."],
  ["off", "Off", "The coach logs calls for Review but stays silent in the ride."],
];

function DistanceField() {
  const d = useStore(distanceM);
  return (
    <label className="field">
      <span>Screen distance</span>
      <select value={d} onChange={(e) => setDistance(Number(e.target.value))}>
        {[0.6, 0.8, 1.0, 1.2, 1.5, 2.0].map((m) => (
          <option key={m} value={m}>
            {m.toFixed(1)} m
          </option>
        ))}
      </select>
      <small>Type sizes follow the distance and your display.</small>
    </label>
  );
}

const ACTION_LABEL: Record<Action, string> = { coach: "Coach (accept · choose)", keep: "Keep (decline · back to plan)", end: "End ride (asks first)", symptom: "Report a symptom (stops the workout)", pause: "Pause / resume", skip: "Skip block", minus: "Intensity −5 %", plus: "Intensity +5 %" };
const SOUND_LABEL: Record<SoundEvent, [string, () => void]> = {
  block: ["Block change", () => play.block(true)],
  countdown: ["Last 3 seconds", () => play.tick()],
  coach: ["Coach asks", () => play.ask()],
  applied: ["Change applied / undone", () => play.applied()],
  warning: ["Sensor lost, ERG released", () => play.warning()],
  reconnected: ["Reconnected", () => play.reconnected()],
  wish: ["Wish answered (granted / changed)", () => play.wish(true)],
};

function ControlsForm() {
  const saved = useStore(bindings);
  const snd = useStore(sound);
  const [learning, setLearning] = useState<Action | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!learning) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      setLearning(null);
      setError(saveBindings({ ...saved, [learning]: [...new Set([e.key, ...saved[learning]])] }));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [learning, saved]);
  const learn = (a: Action) => setLearning(a);
  return (
    <>
      <h2>Controls</h2>
      <p className="muted">Seven ride actions, including Report symptom (stops the workout). Bind a handlebar remote by pressing Learn, then its button. Doing nothing is always safe: questions keep the plan.</p>
      <table className="bindings">
        <tbody>
          {ACTIONS.map((a) => (
            <tr key={a}>
              <td>{ACTION_LABEL[a]}</td>
              <td>
                {saved[a].map((k) => (
                  <kbd key={k}>{keyGlyph(k)}</kbd>
                ))}
              </td>
              <td>
                <button onClick={() => learn(a)}>{learning === a ? "Press a button…" : "Learn"}</button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {error && <p className="status-serious small">⚠ {error}</p>}
      <div className="actions">
        <button onClick={() => setError(saveBindings(DEFAULT_BINDINGS))}>Reset bindings</button>
      </div>
      <h2>Sound</h2>
      <label className="field">
        <span>Volume</span>
        <input type="range" min={0} max={1} step={0.1} value={snd.volume} onChange={(e) => saveSound({ ...snd, volume: Number(e.target.value) })} />
      </label>
      <table className="bindings">
        <tbody>
          {SOUND_EVENTS.map((ev) => {
            const on = !snd.muted.includes(ev);
            return (
              <tr key={ev}>
                <td>{SOUND_LABEL[ev][0]}</td>
                <td>
                  <button role="switch" aria-checked={on} className={on ? "tap wide on" : "tap wide"} onClick={() => saveSound({ ...snd, muted: on ? [...snd.muted, ev] : snd.muted.filter((m) => m !== ev) })}>
                    {on ? "on" : "muted"}
                  </button>
                </td>
                <td>
                  <button onClick={SOUND_LABEL[ev][1]}>Play</button>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </>
  );
}

/**
 * Medical clearance (DESIGN §5.8): lifting a medical stop is a dated, attributed record the rider
 * types in — never a one-click switch. Every change is kept in a log; the daily check-in re-asks.
 */
function HealthForm() {
  const fx = useStore(sceneFixture);
  const [date, setDate] = useState("");
  const [by, setBy] = useState("");
  const [ack, setAck] = useState(false);
  const [log, setLog] = useState<Clearance[]>([]);
  const today = "2026-10-13";
  const valid = /^\d{4}-\d{2}-\d{2}$/.test(date) && date <= today && by.trim().length >= 3 && ack;
  return (
    <>
      <h2>Health</h2>
      {fx.medicalStop || log.length === 0 ? (
        <p className="cap-note stop-note" role="status">
          <b>Medical stop active</b> since 13 Oct — you answered "yes" to chest pain or pressure. crank proposes no plans and the coach is off. Free rides stay possible as a logged choice.
        </p>
      ) : (
        <p className="status-good">✓ Clearance recorded. Plans resume; the daily check-in keeps asking.</p>
      )}
      <h3>Record a medical clearance</h3>
      <div className="fields">
        <label className="field">
          <span>Date of the check</span>
          <input type="date" max={today} value={date} onChange={(e) => setDate(e.target.value)} />
          <small>not in the future</small>
        </label>
        <label className="field">
          <span>Cleared by</span>
          <input value={by} onChange={(e) => setBy(e.target.value)} placeholder="e.g. Dr. name, practice" />
          <small>who examined you</small>
        </label>
      </div>
      <label className="check">
        <input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} /> A doctor examined me for these symptoms and cleared me for hard exercise.
      </label>
      <div className="actions">
        <button className="primary" disabled={!valid} onClick={() => setLog((l) => [...l, { date, by: by.trim(), note: "cleared for hard exercise", recordedAt: today }])}>
          Record clearance
        </button>
      </div>
      <h3>Log</h3>
      <table>
        <tbody>
          <tr>
            <td>13 Oct</td>
            <td>Onboarding: "yes" to chest pain or pressure → medical stop</td>
          </tr>
          {log.map((c) => (
            <tr key={c.recordedAt + c.date}>
              <td>{c.recordedAt}</td>
              <td>
                Clearance recorded: checked {c.date} by {c.by}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
