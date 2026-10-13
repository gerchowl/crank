// Today (DESIGN §5.1) and the Ready check (§5.4).
import { useEffect, useState } from "react";
import type { Checkin, Proposal } from "@crank/shared";
import { Profile } from "../charts";
import { editor } from "../editor/model";
import { blockName, mmss } from "../format";
import { ageDays, athleteModel, calibration, CAP_PCT, cappedProposal, devices, lastCheckin, proposalReasons, sceneFixture, STALE_DAYS, week } from "../mock/ext";
import { backend, loadRide, navigate, noStatus, settle, useSession } from "../session";
import { useStore } from "../store";

type Score = 1 | 2 | 3 | 4 | 5;
type Row = "sleep" | "legs" | "stress" | "motivation";
/** Words left → right always run worse → better; `values` maps each position to the contract's 1–5. */
const ROWS: { key: Row; label: string; words: string[]; values: Score[] }[] = [
  { key: "sleep", label: "Sleep", words: ["poor", "short", "ok", "good", "great"], values: [1, 2, 3, 4, 5] },
  { key: "legs", label: "Legs", words: ["dead", "heavy", "ok", "good", "fresh"], values: [1, 2, 3, 4, 5] },
  { key: "stress", label: "Stress", words: ["very high", "high", "some", "low", "none"], values: [5, 4, 3, 2, 1] },
  { key: "motivation", label: "Motivation", words: ["none", "low", "ok", "keen", "fired up"], values: [1, 2, 3, 4, 5] },
];
const MINUTES = [30, 45, 60, 90];
const word = (row: Row, v: number) => {
  const r = ROWS.find((x) => x.key === row);
  return r?.words[r.values.indexOf(v as Score)] ?? "";
};

function start(p: Proposal) {
  loadRide(p.workout);
  navigate("ready");
}

export function Today() {
  const [c, setC] = useState<Checkin>({ ...lastCheckin });
  const [confirmed, setConfirmed] = useState<"same" | "changed" | null>(null);
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [pending, setPending] = useState(true);
  const fx = useStore(sceneFixture);
  const ready = !!proposal && !pending && confirmed !== null;

  useEffect(() => {
    let live = true;
    setPending(true);
    const capped = sceneFixture.get().healthCap;
    void settle(backend.planner.propose(c)).then((p) => {
      if (!live) return;
      // the planner respects the health cap (stand-in until the planner knows about it)
      setProposal(capped ? cappedProposal(backend.athlete.ftp, c.minutes) : p);
      setPending(false);
    });
    return () => {
      live = false;
    };
  }, [c]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Enter" || !ready || !proposal || e.target instanceof HTMLButtonElement) return;
      e.preventDefault(); // the Ready check's focused button must not receive this same Enter
      start(proposal);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [proposal, ready]);

  const set = (patch: Partial<Checkin>) => {
    setConfirmed("changed");
    setC((prev) => ({ ...prev, ...patch }));
  };
  const compiled = proposal ? backend.compileWorkout(proposal.workout) : null;
  const cp = athleteModel[0];

  return (
    <div className="page today" data-pending={pending || undefined}>
      <header className="page-head">
        <h1>Tuesday</h1>
        <p className="muted">13 October</p>
      </header>
      <div className="today-grid">
        <div className="col">
          <section className="card checkin" aria-label="How are you today">
            <h2>How are you?</h2>
            <p className={confirmed ? "muted small" : "small status-info"}>{confirmed === "same" ? "Same as yesterday — confirmed." : confirmed === "changed" ? "Your answers for today." : "Pre-filled from yesterday. Confirm or change."}</p>
            {ROWS.map((r) => (
              <div className="scale-row" key={r.key} role="radiogroup" aria-label={`${r.label}, worse to better`}>
                <span className="scale-label">{r.label}</span>
                {r.words.map((w, i) => {
                  const v = r.values[i] as Score;
                  return (
                    <button key={w} role="radio" aria-checked={c[r.key] === v} className={c[r.key] === v ? "tap word on" : "tap word"} onClick={() => set({ [r.key]: v })}>
                      {w}
                    </button>
                  );
                })}
              </div>
            ))}
            <div className="scale-row" role="radiogroup" aria-label="Time available">
              <span className="scale-label">Time</span>
              {MINUTES.map((m) => (
                <button key={m} role="radio" aria-checked={c.minutes === m} className={c.minutes === m ? "tap word on" : "tap word"} onClick={() => set({ minutes: m })}>
                  {m} min
                </button>
              ))}
            </div>
            <button className={confirmed === "same" ? "confirm on" : "confirm"} disabled={confirmed === "changed"} onClick={() => setConfirmed("same")}>
              ✓ Same as yesterday
            </button>
          </section>
          <WeekCard />
        </div>

        <div className="col">
          {fx.healthCap && (
            <section className="card cap status-warning" role="status" aria-label="Intensity cap">
              <b>⚠ Intensity capped at endurance ({CAP_PCT.endurance} % FTP, {Math.round((backend.athlete.ftp * CAP_PCT.endurance) / 100)} W)</b>
              <span className="muted"> — you reported chest pain on exertion. Get a medical check before hard efforts; the cap lifts when you confirm it.</span>
              <div className="actions">
                <button>Confirm check</button>
              </div>
            </section>
          )}
          <section className="card proposal" aria-label="Today's proposal" aria-busy={pending}>
            <div className="proposal-head">
              <span className="eyebrow">Coach proposes {pending && <span className="pending-dot">· re-planning…</span>}</span>
              <h2>{proposal?.workout.name ?? " "}</h2>
              <p className="big-meta">{compiled ? `${mmss(compiled.totalS)} · TSS ${Math.round(compiled.tss)} · IF ${compiled.if.toFixed(2)}` : " "}</p>
            </div>
            <div className="proposal-profile">{compiled && <Profile blocks={compiled.blocks} zones={backend.zones.power} height={100} capW={fx.healthCap ? (backend.athlete.ftp * CAP_PCT.endurance) / 100 : undefined} label="Proposed workout profile" />}</div>
            <h3>Why today</h3>
            <ul className="reasons">
              {proposal &&
                proposalReasons(proposal, c.minutes, word("legs", c.legs), word("sleep", c.sleep), !!fx.healthCap).map((r) => <li key={r}>{r}</li>)}
              {fx.healthCap && <li>Every target stays under the dashed cap line.</li>}
              <li className="muted">
                Planned on CP {cp?.value} ± {cp?.sd} W ({calibration.confidence} confidence, {calibration.rides} rides).
              </li>
            </ul>
            <div className="actions">
              <button className="primary big" disabled={!ready} onClick={() => proposal && start(proposal)}>
                Start <kbd>Enter</kbd>
              </button>
              <button disabled={c.minutes <= 30} onClick={() => set({ minutes: MINUTES[Math.max(0, MINUTES.indexOf(c.minutes) - 1)] ?? 30 })}>
                Shorter
              </button>
              <button disabled title="Needs planner variants">
                Easier
              </button>
              <button disabled title="Needs planner variants">
                Harder
              </button>
              <button disabled={!proposal} onClick={() => proposal && (editor.load(proposal.workout), navigate("workouts"))}>
                Edit
              </button>
            </div>
            {!confirmed && <p className="small muted">Confirm how you feel to start.</p>}
          </section>

          <section className="card model" aria-label="Athlete model">
            <h2>Your model</h2>
            {athleteModel.slice(0, 3).map((e) => {
              const age = ageDays(e.updated);
              return (
                <div key={e.label} className={age >= STALE_DAYS ? "estimate stale" : "estimate"}>
                  <span>
                    {e.label}
                    <small className="prov">
                      {e.source === "prior" ? "prior" : `from ${e.n} ${e.source}`} · {age === 0 ? "today" : `${age} d ago`}
                    </small>
                  </span>
                  <b>
                    {e.value} <span className="pm">± {e.sd}</span> <span className="unit">{e.unit}</span>
                  </b>
                </div>
              );
            })}
            <p className="muted small">Calibrating: {calibration.unlocksNext}.</p>
          </section>
        </div>
      </div>
    </div>
  );
}

function WeekCard() {
  return (
    <section className="card week" aria-label="Last 7 days">
      <h2>Last 7 days</h2>
      <div className="week-row">
        {week.map((d) => (
          <div key={d.day} className={`day day-${d.kind}`}>
            <span>{d.day}</span>
            <b>{d.kind === "rest" ? "rest" : d.kind === "today" ? "today" : d.tss}</b>
            {d.kind === "done" && <small className="tss">TSS</small>}
          </div>
        ))}
      </div>
      <p className="muted small">TSS per day · 164 over 7 days</p>
    </section>
  );
}

export function Ready() {
  const s = useSession();
  const raw = useStore(s?.status ?? noStatus);
  const fx = useStore(sceneFixture);
  if (!s || !raw) return <p className="empty">Nothing to ride. Pick a workout on Today.</p>;
  const status = { ...raw, ...fx.status };
  const compiled = backend.compileWorkout(s.workout);
  const trainerOk = status.trainer === "connected";
  const hrOk = status.hrSensor === "connected";
  const go = () => {
    backend.ride.start();
    navigate("ride");
  };
  const first = compiled.blocks[0];
  return (
    <div className="page ready">
      <section className="card ready-card" aria-label="Ready check">
        <span className="eyebrow">Ready check</span>
        <h1>{s.workout.name}</h1>
        <p className="big-meta">
          {mmss(compiled.totalS)} · starts with {first ? `${mmss(first.durS)} ${blockName(s.workout.nodes, first).toLowerCase()}` : "—"}
        </p>
        <ul className="checks">
          <li className={trainerOk ? "status-good" : "status-serious"}>
            <span aria-hidden>{trainerOk ? "✓" : "⚠"}</span>
            <span>
              <b>{devices[0]?.name}</b> {trainerOk ? `${devices[0]?.protocol} · connected, ERG ready` : "not found — is it powered, and are Zwift or the Tacx app closed? It accepts one connection."}
            </span>
            {!trainerOk && <button>Retry</button>}
          </li>
          <li className={hrOk ? "status-good" : "status-warning"}>
            <span aria-hidden>{hrOk ? "✓" : "…"}</span>
            <span>
              <b>{devices[1]?.name}</b>{" "}
              {hrOk ? "· connected" : "not found yet — still scanning. Wet the strap's contacts. If you start without it, it joins as soon as it's found; until then HR-guided recoveries run to their maximum and the coach can't push."}
            </span>
            {!hrOk && <button autoFocus>Retry now</button>}
          </li>
          {fx.healthCap && (
            <li className="status-warning">
              <span aria-hidden>⚠</span>
              <span>
                <b>Capped at {CAP_PCT.endurance} % FTP</b> until your medical check is confirmed
              </span>
            </li>
          )}
          <li className="muted">
            <span aria-hidden>i</span>
            <span>Mode ERG · the trainer holds the target; keep your cadence up</span>
          </li>
        </ul>
        <div className="actions">
          <button className="primary big" autoFocus={hrOk} disabled={!trainerOk} onClick={go}>
            {hrOk ? "Start riding" : "Start without HR"}
          </button>
          <button onClick={() => navigate("today")}>Back</button>
        </div>
      </section>
    </div>
  );
}
