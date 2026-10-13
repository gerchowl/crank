// Today (DESIGN §5.1) and the Ready check (§5.4).
import { useEffect, useState } from "react";
import type { Checkin, Proposal } from "@crank/shared";
import { Profile } from "../charts";
import { blockName, mmss } from "../format";
import { athleteModel, calibration, devices, lastCheckin, week } from "../mock/ext";
import { editor } from "../editor/model";
import { backend, loadRide, navigate, noStatus, settle, useSession } from "../session";
import { useStore } from "../store";

type Score = 1 | 2 | 3 | 4 | 5;
const ROWS: { key: "sleep" | "legs" | "stress" | "motivation"; label: string; lo: string; hi: string }[] = [
  { key: "sleep", label: "Sleep", lo: "poor", hi: "great" },
  { key: "legs", label: "Legs", lo: "heavy", hi: "fresh" },
  { key: "stress", label: "Stress", lo: "calm", hi: "high" },
  { key: "motivation", label: "Motivation", lo: "low", hi: "keen" },
];
const MINUTES = [30, 45, 60, 90];

function start(p: Proposal) {
  loadRide(p.workout);
  navigate("ready");
}

export function Today() {
  const [c, setC] = useState<Checkin>({ ...lastCheckin });
  const [touched, setTouched] = useState(false);
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [pending, setPending] = useState(true);

  useEffect(() => {
    let live = true;
    setPending(true);
    void settle(backend.planner.propose(c)).then((p) => {
      if (!live) return;
      setProposal(p);
      setPending(false);
    });
    return () => {
      live = false;
    };
  }, [c]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Enter" || !proposal || pending || e.target instanceof HTMLButtonElement) return;
      e.preventDefault(); // the Ready check's focused button must not receive this same Enter
      start(proposal);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [proposal, pending]);

  const set = (patch: Partial<Checkin>) => {
    setTouched(true);
    setC((prev) => ({ ...prev, ...patch }));
  };
  const compiled = proposal ? backend.compileWorkout(proposal.workout) : null;
  const cp = athleteModel[0];

  return (
    <div className="page today" data-pending={pending || undefined}>
      <header className="page-head">
        <h1>Tuesday</h1>
        <p className="muted">13 October · {touched ? "your answers for today" : "pre-filled from yesterday — change what's different"}</p>
      </header>
      <div className="today-grid">
        <div className="col">
          <section className="card checkin" aria-label="How are you today">
            <h2>How are you?</h2>
            {ROWS.map((r) => (
              <div className="scale-row" key={r.key} role="radiogroup" aria-label={r.label}>
                <span className="scale-label">{r.label}</span>
                <span className="scale-end">{r.lo}</span>
                {([1, 2, 3, 4, 5] as Score[]).map((v) => (
                  <button key={v} role="radio" aria-checked={c[r.key] === v} className={c[r.key] === v ? "tap on" : "tap"} onClick={() => set({ [r.key]: v })}>
                    {v}
                  </button>
                ))}
                <span className="scale-end">{r.hi}</span>
              </div>
            ))}
            <div className="scale-row" role="radiogroup" aria-label="Time available">
              <span className="scale-label">Time</span>
              {MINUTES.map((m) => (
                <button key={m} role="radio" aria-checked={c.minutes === m} className={c.minutes === m ? "tap wide on" : "tap wide"} onClick={() => set({ minutes: m })}>
                  {m}′
                </button>
              ))}
            </div>
          </section>
          <WeekCard />
        </div>

        <div className="col">
          <section className="card proposal" aria-label="Today's proposal" aria-busy={pending}>
            <div className="proposal-head">
              <span className="eyebrow">Coach proposes {pending && <span className="pending-dot">· re-planning…</span>}</span>
              <h2>{proposal?.workout.name ?? " "}</h2>
              <p className="big-meta">
                {compiled ? `${mmss(compiled.totalS)} · TSS ${Math.round(compiled.tss)} · IF ${compiled.if.toFixed(2)}` : " "}
              </p>
            </div>
            <div className="proposal-profile">{compiled && <Profile blocks={compiled.blocks} zones={backend.zones.power} height={110} label="Proposed workout profile" />}</div>
            <p className="why">
              <b>Why today:</b> {proposal?.rationale ?? " "}
            </p>
            <p className="muted small">
              Planned on CP {cp?.value} ± {cp?.sd} W ({calibration.confidence} confidence, {calibration.rides} rides).
            </p>
            <div className="actions">
              <button className="primary big" disabled={!proposal || pending} onClick={() => proposal && start(proposal)}>
                Start <kbd>Enter</kbd>
              </button>
              <button disabled={c.minutes <= 30} onClick={() => set({ minutes: MINUTES[Math.max(0, MINUTES.indexOf(c.minutes) - 1)] ?? 30 })}>
                Shorter
              </button>
              <button disabled title="Needs planner variants (#15)">
                Easier
              </button>
              <button disabled title="Needs planner variants (#15)">
                Harder
              </button>
              <button disabled={!proposal} onClick={() => proposal && (editor.load(proposal.workout), navigate("workouts"))}>
                Edit
              </button>
            </div>
          </section>

          <section className="card model" aria-label="Athlete model">
            <h2>Your model</h2>
            {athleteModel.slice(0, 3).map((e) => (
              <p key={e.label} className="estimate">
                <span>{e.label}</span>
                <b>
                  {e.value} <small>± {e.sd} {e.unit}</small>
                </b>
              </p>
            ))}
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
          </div>
        ))}
      </div>
      <p className="muted small">TSS per day · 164 over 7 days</p>
    </section>
  );
}

export function Ready() {
  const s = useSession();
  const status = useStore(s?.status ?? noStatus);
  if (!s || !status) return <p className="empty">Nothing to ride. Pick a workout on Today.</p>;
  const compiled = backend.compileWorkout(s.workout);
  const trainerOk = status.trainer === "connected";
  const hrOk = status.hrSensor === "connected";
  const start = () => {
    backend.ride.start();
    navigate("ride");
  };
  const first = compiled.blocks[0];
  return (
    <div className="page ready" onKeyDown={(e) => e.key === " " && trainerOk && start()}>
      <section className="card ready-card" aria-label="Ready check">
        <span className="eyebrow">Ready check</span>
        <h1>{s.workout.name}</h1>
        <p className="big-meta">
          {mmss(compiled.totalS)} · starts with {first ? `${mmss(first.durS)} ${blockName(s.workout.nodes, first).toLowerCase()}` : "—"}
        </p>
        <ul className="checks">
          <li className={trainerOk ? "status-good" : "status-warning"}>
            <span aria-hidden>{trainerOk ? "✓" : "…"}</span> <b>{devices[0]?.name}</b> {devices[0]?.protocol} · {trainerOk ? "connected, ERG ready" : "connecting"}
          </li>
          <li className={hrOk ? "status-good" : "status-warning"}>
            <span aria-hidden>{hrOk ? "✓" : "!"}</span> <b>{devices[1]?.name}</b> · {hrOk ? "connected" : "not found — you can ride without HR; HR-based coaching will be off"}
          </li>
          <li className="muted">
            <span aria-hidden>i</span> Mode ERG · the trainer holds the target; keep your cadence up
          </li>
        </ul>
        <div className="actions">
          <button className="primary big" autoFocus disabled={!trainerOk} onClick={start}>
            Start riding <kbd>Space</kbd>
          </button>
          <button onClick={() => navigate("today")}>Back</button>
        </div>
      </section>
    </div>
  );
}
