// Training profile (DESIGN §15.1): what the rider trains for, as the planner's prior — one form in
// Settings and in onboarding. It shows what it changes, with the evidence for each line.
import { useState } from "react";
import { FOCI, FORMAT_NAME, FORMATS, profileEffects, profileFixture, type TrainingProfile } from "../mock/wish";
import { GradeChip, WhyChip } from "./Why";

const weeksTo = (date: string) => Math.round((Date.parse(date) - Date.UTC(2026, 9, 13)) / (7 * 86400000));

export function ProfileForm({ inOnboarding = false }: { inOnboarding?: boolean }) {
  const [saved, setSaved] = useState<TrainingProfile>(profileFixture);
  const [p, setP] = useState<TrainingProfile>(profileFixture);
  const dirty = JSON.stringify(p) !== JSON.stringify(saved);
  const set = (patch: Partial<TrainingProfile>) => setP((x) => ({ ...x, ...patch }));
  const Title = inOnboarding ? "h1" : "h2";
  return (
    <div className="profile">
      <div>
        <Title>What are you training for?</Title>
        <p className="muted">A starting point for the mix of sessions — not a fixed plan. Today's check-in and your wish still decide each ride. Change it any time.</p>
        <div className="choices column" role="radiogroup" aria-label="Focus">
          {FOCI.map(([id, label, says]) => (
            <button key={id} role="radio" aria-checked={p.focus === id} className={p.focus === id ? "tap wide on" : "tap wide"} onClick={() => set({ focus: id })}>
              <b>{label}</b>
              <span>{says}</span>
            </button>
          ))}
        </div>
        <h3>Event (optional)</h3>
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
        <h3>Your week</h3>
        <div className="choices" role="radiogroup" aria-label="Days per week">
          {[2, 3, 4, 5, 6].map((d) => (
            <button key={d} role="radio" aria-checked={p.daysPerWeek === d} className={p.daysPerWeek === d ? "tap on" : "tap"} onClick={() => set({ daysPerWeek: d })}>
              {d} days
            </button>
          ))}
        </div>
        <label className="field">
          <span>Hours per week</span>
          <input inputMode="numeric" value={p.hoursPerWeek} onChange={(e) => set({ hoursPerWeek: Number(e.target.value) || 0 })} />
          <small>a budget, not a target: each day's time comes from Today</small>
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
          <input type="checkbox" checked={!!p.returning} onChange={(e) => set({ returning: e.target.checked ? { since: "2026-10-13" } : null })} /> Coming back after a break (2+ weeks off): start with a return block. It ends by itself once your numbers settle.
        </label>
      </div>
      <aside className="card effects" aria-label="What this changes">
        <h3>What this changes</h3>
        <ul className="reasons">
          {profileEffects(p).map((r) => (
            <li key={r.text}>
              {r.text} {r.rule ? <WhyChip id={r.rule} /> : <GradeChip g="D" />}
            </li>
          ))}
        </ul>
        {!inOnboarding && (
          <div className="actions">
            <button className="primary" disabled={!dirty} onClick={() => setSaved(p)}>
              Save
            </button>
            <button disabled={!dirty} onClick={() => setP(saved)}>
              Cancel
            </button>
          </div>
        )}
      </aside>
    </div>
  );
}
