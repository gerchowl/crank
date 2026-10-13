// Onboarding (DESIGN §5.8, spec/ONBOARDING.md): six resumable steps ending on the starting model.
import { useState } from "react";
import { devices, priorModel } from "../mock/ext";
import { navigate, useRoute } from "../session";

const STEPS = [
  ["about", "About you"],
  ["background", "Background"],
  ["numbers", "Current numbers"],
  ["health", "Health check"],
  ["devices", "Devices"],
  ["baseline", "Baseline"],
  ["result", "Your starting model"],
] as const;
type StepId = (typeof STEPS)[number][0];

function Choice({ options, value, onPick, label }: { options: string[]; value: string | null; onPick: (v: string) => void; label: string }) {
  return (
    <div className="choices" role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button key={o} role="radio" aria-checked={value === o} className={value === o ? "tap wide on" : "tap wide"} onClick={() => onPick(o)}>
          {o}
        </button>
      ))}
    </div>
  );
}

export function Onboarding() {
  const r = useRoute();
  const initial = STEPS.findIndex(([id]) => id === r.sub);
  const [idx, setIdx] = useState(initial < 0 ? 0 : initial);
  const [a, setA] = useState<Record<string, string>>({ sex: "prefer not to say", years: "5–10", structured: "some", ftpKnown: "yes", ftpFrom: "20-min test", path: "passive" });
  const step = STEPS[idx]?.[0] ?? "about";
  const pick = (k: string) => (v: string) => setA((p) => ({ ...p, [k]: v }));
  const field = (k: string, label: string, unit: string, value: string) => (
    <label className="field">
      <span>{label}</span>
      <input inputMode="numeric" defaultValue={value} onChange={(e) => setA((p) => ({ ...p, [k]: e.target.value }))} />
      <small>{unit}</small>
    </label>
  );
  const body: Record<StepId, React.ReactNode> = {
    about: (
      <>
        <h1>About you</h1>
        <p className="muted">These set the starting priors. Everything is editable later.</p>
        <div className="fields">
          {field("age", "Age", "years", "41")}
          {field("weight", "Weight", "kg", "72")}
          {field("rest", "Resting HR", "bpm · or measure on the first ride", "52")}
          {field("max", "Max HR, if known", "bpm", "")}
        </div>
        <h3>Sex (optional, adjusts priors)</h3>
        <Choice label="Sex" options={["female", "male", "prefer not to say"]} value={a.sex ?? null} onPick={pick("sex")} />
      </>
    ),
    background: (
      <>
        <h1>Background</h1>
        <h3>Years riding</h3>
        <Choice label="Years" options={["< 2", "2–5", "5–10", "> 10"]} value={a.years ?? null} onPick={pick("years")} />
        <h3>Structured training before?</h3>
        <Choice label="Structured" options={["none", "some", "lots"]} value={a.structured ?? null} onPick={pick("structured")} />
        <div className="fields">{field("hours", "Typical hours per week (last 3 months)", "h", "6")}</div>
      </>
    ),
    numbers: (
      <>
        <h1>Current numbers</h1>
        <p className="muted">Unknown is fine — crank starts conservative and learns from your rides. No test needed today.</p>
        <h3>Do you know your FTP or CP?</h3>
        <Choice label="Known" options={["yes", "roughly", "no"]} value={a.ftpKnown ?? null} onPick={pick("ftpKnown")} />
        <div className="fields">{field("ftp", "FTP / CP", "W", "255")}</div>
        <h3>From what?</h3>
        <Choice label="Source" options={["ramp test", "20-min test", "app estimate", "guess"]} value={a.ftpFrom ?? null} onPick={pick("ftpFrom")} />
        <p className="muted small">A 20-min test from 2 months ago counts as a self-report: ± 35 W until rides confirm it.</p>
      </>
    ),
    health: (
      <>
        <h1>Health check</h1>
        <p className="muted">Any "yes" caps intensity until you confirm a medical check. This is not a diagnosis.</p>
        {["Heart condition or chest pain on exertion?", "Dizziness or fainting?", "Medication affecting heart rate?", "Injury that limits hard efforts?"].map((q) => (
          <div key={q} className="yn">
            <span>{q}</span>
            <Choice label={q} options={["no", "yes"]} value={a[q] ?? "no"} onPick={pick(q)} />
          </div>
        ))}
      </>
    ),
    devices: (
      <>
        <h1>Devices</h1>
        <p className="muted">Pedal a few strokes to test. The NEO accepts one connection at a time — close Zwift or the Tacx app first.</p>
        <ul className="devices">
          {devices.map((d) => (
            <li key={d.id} className={`device ${d.state}`}>
              <span className="role">{d.role}</span>
              <b>{d.name}</b>
              <span className="muted protocol">{d.protocol}</span>
              <span className={d.state === "connected" ? "status-good" : "muted"}>{d.state === "connected" ? "✓ paired · signal ok" : "available"}</span>
              <button disabled={d.state === "connected"}>{d.state === "connected" ? "Paired" : "Pair"}</button>
            </li>
          ))}
        </ul>
      </>
    ),
    baseline: (
      <>
        <h1>How should crank learn your numbers?</h1>
        <Choice label="Baseline path" options={["import intervals.icu", "assessment week", "passive"]} value={a.path ?? null} onPick={pick("path")} />
        <ul className="paths">
          <li>
            <b>Import</b> — 6+ weeks with power give a confident model on day 1 (connector later).
          </li>
          <li>
            <b>Assessment week</b> — 2–3 short sessions with maximal efforts (3′ and 12′ on separate days). Confident after ~1 week.
          </li>
          <li>
            <b>Passive</b> — just ride; short max efforts are added to the end of normal workouts. Converges over 2–4 weeks.
          </li>
        </ul>
      </>
    ),
    result: (
      <>
        <h1>Your starting model</h1>
        <p className="muted">Low confidence — the coach targets the conservative side and asks "how did that feel?" after hard blocks.</p>
        <table className="estimates">
          <thead>
            <tr>
              <th>estimate</th>
              <th>value ± sd</th>
              <th>relative uncertainty</th>
              <th>source</th>
            </tr>
          </thead>
          <tbody>
            {priorModel.map((e) => (
              <tr key={e.label}>
                <td>{e.label}</td>
                <td className="num">
                  <b>{e.value}</b> ± {e.sd} {e.unit}
                </td>
                <td className="uncertainty">
                  <i style={{ width: `${Math.min(100, (e.sd / e.value) * 400)}%` }} />
                </td>
                <td className="muted">{e.source.replace("_", "-")}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <p>
          <b>What narrows it:</b> your next 3–4 rides (passive learning). <b>Auto-push</b> and aggressive replans unlock at medium confidence.
        </p>
      </>
    ),
  };
  const last = idx === STEPS.length - 1;
  return (
    <div className="page onboarding">
      <ol className="steps" aria-label="Steps">
        {STEPS.map(([id, label], i) => (
          <li key={id} className={i === idx ? "now" : i < idx ? "done" : ""}>
            {label}
          </li>
        ))}
      </ol>
      <section className="card onboarding-card">{body[step]}</section>
      <div className="actions">
        {idx > 0 && <button onClick={() => setIdx(idx - 1)}>Back</button>}
        <span className="spacer" />
        {!last && step !== "result" && (
          <button onClick={() => setIdx(idx + 1)} title="Skipped sections keep wide priors">
            Skip
          </button>
        )}
        <button className="primary big" onClick={() => (last ? navigate("today") : setIdx(idx + 1))}>
          {last ? "Ride today" : "Continue"}
        </button>
      </div>
    </div>
  );
}
