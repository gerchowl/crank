// Onboarding (DESIGN §5.8, spec/ONBOARDING.md): a mode without the rail; progress is saved after
// every step so a relaunch resumes where the rider left; every Skip states its consequence.
import { useState, type ReactNode } from "react";
import { devices, priorModel } from "../mock/ext";
import { ONBOARDING_STEPS, onboardingStep, saveOnboardingStep, type OnboardingStep } from "../persist";
import { navigate, useRoute } from "../session";
import { useStore } from "../store";

const TITLE: Record<OnboardingStep, string> = {
  about: "About you",
  background: "Background",
  numbers: "Current numbers",
  health: "Health check",
  devices: "Devices",
  baseline: "Baseline",
  result: "Your starting model",
};
const SKIP: Record<OnboardingStep, string> = {
  about: "Skipping uses population priors: CP ± 60 W, HR max from age unknown → HR zones off until a hard ride.",
  background: "Skipping assumes a beginner: the first week stays easy.",
  numbers: "Skipping starts CP at 220 ± 60 W: targets stay conservative until rides narrow it.",
  health: "Skipping counts as unanswered: intensity stays capped at threshold until you answer.",
  devices: "Skipping: you can pair before the first ride, on the Ready check.",
  baseline: "Skipping chooses passive learning.",
  result: "",
};
const HEALTH_QS = ["Heart condition or chest pain on exertion?", "Dizziness or fainting?", "Medication affecting heart rate?", "Injury that limits hard efforts?"];

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
  const saved = useStore(onboardingStep);
  const fromRoute = ONBOARDING_STEPS.find((s) => s === r.sub);
  const resuming = r.sub === "resume" || (!fromRoute && saved !== null);
  const [idx, setIdx] = useState(Math.max(0, ONBOARDING_STEPS.indexOf(fromRoute ?? (r.sub === "resume" ? "health" : (saved ?? "about")))));
  const [welcome, setWelcome] = useState(resuming);
  const [a, setA] = useState<Record<string, string>>({ sex: "prefer not to say", years: "5–10", structured: "some", ftpKnown: "yes", ftpFrom: "20-min test", path: "passive learning", [HEALTH_QS[2] ?? ""]: r.sub === "health" ? "yes" : "no" });
  const step = ONBOARDING_STEPS[idx] ?? "about";
  const go = (i: number) => {
    const next = ONBOARDING_STEPS[i];
    if (!next) return;
    setIdx(i);
    saveOnboardingStep(next);
  };
  const finish = () => {
    saveOnboardingStep(null);
    navigate("today");
  };
  const pick = (k: string) => (v: string) => setA((p) => ({ ...p, [k]: v }));
  const field = (k: string, label: string, unit: string, value: string, type = "text") => (
    <label className="field">
      <span>{label}</span>
      <input type={type} inputMode={type === "text" ? "numeric" : undefined} defaultValue={value} onChange={(e) => setA((p) => ({ ...p, [k]: e.target.value }))} />
      <small>{unit}</small>
    </label>
  );
  const capped = HEALTH_QS.some((q) => a[q] === "yes");
  const bodies: Record<OnboardingStep, ReactNode> = {
    about: (
      <>
        <h1>About you</h1>
        <p className="muted">These set the starting priors. Everything is editable later.</p>
        <div className="fields">
          {field("age", "Age", "years", "41")}
          {field("weight", "Weight", "kg", "72")}
          {field("rest", "Resting HR", "bpm · or measured on the first ride", "52")}
          {field("max", "Max HR, if known", "bpm · unknown is fine", "")}
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
        <div className="fields">
          {field("hours", "Typical hours per week, last 3 months", "h", "6")}
          {field("longest", "Longest ride in the last month", "h", "3")}
        </div>
      </>
    ),
    numbers: (
      <>
        <h1>Current numbers</h1>
        <p className="muted">Unknown is fine — crank starts conservative and learns from your rides. No test needed today.</p>
        <h3>Do you know your FTP or CP?</h3>
        <Choice label="Known" options={["yes", "roughly", "no"]} value={a.ftpKnown ?? null} onPick={pick("ftpKnown")} />
        <div className="fields">
          {field("ftp", "FTP / CP", "W", "255")}
          {field("ftpDate", "Measured on", "older values count as less certain", "2026-08-10", "date")}
        </div>
        <h3>From what?</h3>
        <Choice label="Source" options={["ramp test", "20-min test", "app estimate", "guess"]} value={a.ftpFrom ?? null} onPick={pick("ftpFrom")} />
        <p className="small status-info">→ 255 W from a 20-min test 2 months old starts as 255 ± 35 W (self-report) until rides confirm it.</p>
      </>
    ),
    health: (
      <>
        <h1>Health check</h1>
        <p className="muted">A short safety screen. It is not a diagnosis.</p>
        {HEALTH_QS.map((q) => (
          <div key={q} className="yn">
            <span>{q}</span>
            <Choice label={q} options={["no", "yes"]} value={a[q] ?? "no"} onPick={pick(q)} />
          </div>
        ))}
        {capped ? (
          <p className="cap-note status-warning">
            ⚠ <b>Intensity capped at threshold</b> until you confirm a medical check. You'll see the cap on Today, before each ride and in the targets; confirm it in Settings → Athlete model.
          </p>
        ) : (
          <p className="small muted">All "no": no cap.</p>
        )}
      </>
    ),
    devices: (
      <>
        <h1>Devices</h1>
        <p className="muted">Pedal a few strokes: the readings below should move. The NEO accepts one connection at a time — close Zwift or the Tacx app first.</p>
        <ul className="devices">
          {devices.map((d) => (
            <li key={d.id} className={`device ${d.state}`}>
              <span className="role">{d.role}</span>
              <b>{d.name}</b>
              <span className="muted protocol">{d.protocol}</span>
              <span className={d.state === "connected" ? "status-good" : "muted"}>{d.state === "connected" ? `✓ ${d.live}` : "available"}</span>
              <button disabled={d.state === "connected"}>{d.state === "connected" ? "Paired" : "Pair"}</button>
            </li>
          ))}
        </ul>
      </>
    ),
    baseline: (
      <>
        <h1>How should crank learn your numbers?</h1>
        <Choice label="Baseline path" options={["import intervals.icu", "assessment week", "passive learning"]} value={a.path ?? null} onPick={pick("path")} />
        <table className="paths">
          <thead>
            <tr>
              <th />
              <th>what it takes</th>
              <th>confident after</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>Import</td>
              <td>6+ weeks of rides with power from intervals.icu</td>
              <td>day 1</td>
            </tr>
            <tr>
              <td>Assessment week</td>
              <td>2–3 short sessions with maximal 3′ and 12′ efforts on separate days</td>
              <td>~1 week</td>
            </tr>
            <tr>
              <td>Passive learning</td>
              <td>just ride; short max efforts at the end of normal workouts</td>
              <td>2–4 weeks</td>
            </tr>
          </tbody>
        </table>
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
          <b>What narrows it:</b> your next 3–4 rides (passive learning). <b>Auto-applied coach changes</b> stay off; aggressive replans unlock at medium confidence.
        </p>
      </>
    ),
  };
  const last = step === "result";
  return (
    <div className="page onboarding">
      <ol className="steps" aria-label="Steps">
        {ONBOARDING_STEPS.map((id, i) => (
          <li key={id} className={i === idx ? "now" : i < idx ? "done" : ""}>
            {TITLE[id]}
          </li>
        ))}
      </ol>
      {welcome && (
        <section className="card welcome" role="status">
          <b>Welcome back</b> — continue at {TITLE[step]} ({idx + 1} of {ONBOARDING_STEPS.length}). Your earlier answers are kept.
          <button onClick={() => setWelcome(false)}>Continue</button>
          <button onClick={() => (go(0), setWelcome(false))}>Start over</button>
        </section>
      )}
      <section className="card onboarding-card">{bodies[step]}</section>
      <div className="actions">
        {idx > 0 && <button onClick={() => go(idx - 1)}>Back</button>}
        <span className="spacer" />
        {!last && (
          <span className="skip">
            <span className="small muted">{SKIP[step]}</span>
            <button onClick={() => go(idx + 1)}>Skip</button>
          </span>
        )}
        <button className="primary big" onClick={() => (last ? finish() : go(idx + 1))}>
          {last ? "Ride today" : "Continue"}
        </button>
      </div>
    </div>
  );
}
