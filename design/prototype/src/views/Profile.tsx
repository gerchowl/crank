// Training profile (DESIGN §15.1): what the rider trains for, as the planner's prior — one form in Settings
// and in onboarding. "What this changes" compares the saved profile with the draft (before → after), and
// every line carries its evidence and the conditions it needs: a C-grade line is a suggestion, never a promise.
import { useState, type ReactNode } from "react";
import { defaultProfile, FOCI, FORMAT_NAME, FORMATS, profileEffects, profileFixture, RETURN_GAP_DAYS, type Effect, type TrainingProfile } from "../mock/wish";
import { GradeChip, WhyChip } from "./Why";

const weeksTo = (date: string) => Math.round((Date.parse(date) - Date.UTC(2026, 9, 13)) / (7 * 86400000));

function EffectLine({ e, mark }: { e: Effect; mark: "new" | "gone" | null }) {
  return (
    <li className={mark === "gone" ? "gone" : undefined}>
      {mark && <span className={`tag ${mark}`}>{mark === "new" ? "new" : "no longer"}</span>} {e.text} {e.rule ? <WhyChip id={e.rule} /> : <GradeChip g="D" />}
      {e.when && <span className="when small muted"> — when: {e.when}</span>}
    </li>
  );
}

export function ProfileForm({ inOnboarding = false }: { inOnboarding?: boolean }) {
  // onboarding starts from the default (general fitness), with the budget pre-filled from the Background step
  const initial = inOnboarding ? { ...defaultProfile, hoursPerWeek: 6 } : profileFixture;
  const [saved, setSaved] = useState<TrainingProfile>(initial);
  const [p, setP] = useState<TrainingProfile>(inOnboarding ? initial : profileFixture);
  const dirty = JSON.stringify(p) !== JSON.stringify(saved);
  const set = (patch: Partial<TrainingProfile>) => setP((x) => ({ ...x, ...patch }));
  const before = profileEffects(saved);
  const after = profileEffects(p);
  const Title = inOnboarding ? "h1" : "h2";
  const details: ReactNode = (
    <>
      <h3>Event</h3>
      <div className="fields">
        <label className="field">
          <span>Name</span>
          <input value={p.event?.name ?? ""} onChange={(e) => set({ event: { name: e.target.value, date: p.event?.date ?? "" } })} placeholder="none" />
        </label>
        <label className="field">
          <span>Date</span>
          <input type="date" value={p.event?.date ?? ""} onChange={(e) => set({ event: { name: p.event?.name ?? "", date: e.target.value } })} />
          <small>{p.event?.date ? `${weeksTo(p.event.date)} weeks away` : "no date"}</small>
        </label>
      </div>
      <h3>Your week, from now</h3>
      <div className="choices" role="radiogroup" aria-label="Days per week">
        {[2, 3, 4, 5, 6].map((d) => (
          <button key={d} role="radio" aria-checked={p.daysPerWeek === d} className={p.daysPerWeek === d ? "tap on" : "tap"} onClick={() => set({ daysPerWeek: d })}>
            {d} days
          </button>
        ))}
      </div>
      <label className="field">
        <span>Hours you can ride per week</span>
        <span className="unit-input">
          <input inputMode="numeric" value={p.hoursPerWeek} onChange={(e) => set({ hoursPerWeek: Number(e.target.value) || 0 })} /> h / week
        </span>
        <small>{inOnboarding ? "pre-filled from Background (last 3 months); " : ""}a budget, not a target — each day's time comes from Today</small>
      </label>
      <h3>Formats you enjoy</h3>
      <div className="choices" aria-label="Formats you enjoy">
        {FORMATS.map((f) => {
          const on = p.likes.includes(f);
          return (
            <button key={f} role="switch" aria-checked={on} className={on ? "tap wide on" : "tap wide"} onClick={() => set({ likes: on ? p.likes.filter((x) => x !== f) : [...p.likes, f] })}>
              {FORMAT_NAME[f]}
            </button>
          );
        })}
      </div>
      <label className="check">
        <input type="checkbox" checked={!!p.returning} onChange={(e) => set({ returning: e.target.checked ? { since: "2026-10-13" } : null })} /> Coming back after {RETURN_GAP_DAYS}+ days off: start with a return block. It ends by itself once your numbers settle.
      </label>
    </>
  );
  return (
    <div className="profile">
      <div>
        <Title>What are you training for?</Title>
        <p className="muted">A starting point for the mix of sessions — not a fixed plan. Today's check-in and your wish still decide each ride.</p>
        <div className="choices column" role="radiogroup" aria-label="Focus">
          {FOCI.map(([id, label, says]) => (
            <button key={id} role="radio" aria-checked={p.focus === id} className={p.focus === id ? "tap wide on" : "tap wide"} onClick={() => set({ focus: id })}>
              <b>{label}</b>
              <span>{says}</span>
            </button>
          ))}
        </div>
        {inOnboarding ? (
          <details className="more">
            <summary>Add an event, your week and formats (optional)</summary>
            {details}
          </details>
        ) : (
          details
        )}
        {!inOnboarding && (
          <div className="actions">
            <button className="primary" disabled={!dirty} onClick={() => setSaved({ ...p, version: p.version + 1 })}>
              Save as version {p.version + 1}
            </button>
            <button disabled={!dirty} onClick={() => setP(saved)}>
              Cancel
            </button>
            <span className="small muted">Review shows which profile version planned each ride.</span>
          </div>
        )}
      </div>
      <aside className="card effects" aria-label="What this changes">
        <h3>What this changes</h3>
        <p className="small muted">{dirty ? `Before → after: compared with ${inOnboarding ? "the default (general fitness)" : `your saved profile (version ${saved.version})`}.` : inOnboarding ? "With the default, general fitness & health:" : `Your saved profile (version ${saved.version}):`}</p>
        <ul className="reasons">
          {after.map((e) => (
            <EffectLine key={e.id} e={e} mark={dirty && !before.some((b) => b.id === e.id) ? "new" : null} />
          ))}
          {dirty && before.filter((b) => !after.some((e) => e.id === b.id)).map((e) => <EffectLine key={e.id} e={e} mark="gone" />)}
        </ul>
      </aside>
    </div>
  );
}
