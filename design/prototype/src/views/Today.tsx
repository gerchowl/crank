// Today (DESIGN §5.1) and the Ready check (§5.4).
import { useEffect, useRef, useState } from "react";
import type { Checkin, Proposal } from "@crank/shared";
import { Profile } from "../charts";
import { editor } from "../editor/model";
import { blockName, mmss } from "../format";
import { ageDays, athleteModel, calibration, CAP_PCT_CP, cappedProposal, devices, freeRide, lastCheckin, proposalReasons, sceneFixture, STALE_DAYS, week } from "../mock/ext";
import { backend, navigate, noStatus, settle, useRoute, useSession } from "../session";
import { requestStart, safety, stopActive } from "../safety";
import { RedFlagCheck } from "./Gate";
import { useStore } from "../store";
import { KeyHint } from "./Ride";
import { bindings } from "../persist";
import { planWish, proposalFor, stepWish, type Cell, type Wish } from "../mock/wish";
import { WishCard, WishOutcomeBlock } from "./Wish";

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

const cpW = athleteModel[0]?.value ?? backend.athlete.ftp;

/** Every start goes through the shared guard (safety.ts), never straight to the Ready check. */
function start(p: Proposal) {
  requestStart(p.workout, "today");
}

export function Today() {
  const fx0 = sceneFixture.get();
  const [c, setC] = useState<Checkin>({ ...lastCheckin, ...fx0.checkin });
  const [confirmed, setConfirmed] = useState<"same" | "changed" | null>(fx0.checkin ? "changed" : null);
  // the wish (DESIGN §15.2): null = the coach picks; safety is applied by the planner before the wish
  const [wish, setWishState] = useState<Wish>(fx0.wish ?? null);
  const [anyway, setAnyway] = useState(false);
  const wishAt = useRef(0);
  const setWish = (w: Wish) => ((wishAt.current = performance.now()), setAnyway(false), setWishState(w));
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [pending, setPending] = useState(true);
  const fx = useStore(sceneFixture);
  // every check-in re-asks all red flags (no default, DESIGN §5.8); the answer lives in the shared guard
  const safe = useStore(safety);
  const route = useRoute();
  const [freeConfirm, setFreeConfirm] = useState(!!fx.freeConfirm || route.sub === "free");
  const stop = stopActive() || safe.stop;
  const ready = !!proposal && !pending && confirmed !== null && safe.checkedToday;
  const capped = !!fx.healthCap;
  const coachPick: Cell = capped ? { intensity: "tempo", format: "intervals" } : { intensity: "vo2max", format: "intervals" };
  const outcome = planWish(wish, { legs: c.legs, sleep: c.sleep, minutes: c.minutes, capped, coachPick });

  useEffect(() => {
    let live = true;
    setPending(true);
    const cap = sceneFixture.get().healthCap;
    void settle(backend.planner.propose(c)).then((p) => {
      if (!live) return;
      // the planner respects the health cap first, then the wish (stand-ins until the planner knows both)
      const o = planWish(wish, { legs: c.legs, sleep: c.sleep, minutes: c.minutes, capped: !!cap, coachPick });
      setProposal(wish ? proposalFor(anyway && o.overridable ? wish : o.delivered, c.minutes, cpW) : cap ? cappedProposal(cpW, c.minutes) : p);
      setPending(false);
    });
    return () => {
      live = false;
    };
  }, [c, wish, anyway]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement) return;
      const b = bindings.get();
      // saddle gesture: Intensity −/+ move the wish along intensity, Keep returns to the coach's pick (§15.2)
      if (!stop && (b.minus.includes(e.key) || b.plus.includes(e.key))) return setWish(stepWish(wish ?? coachPick, b.minus.includes(e.key) ? -1 : 1, capped));
      if (!stop && b.keep.includes(e.key) && !(e.target instanceof HTMLButtonElement)) return setWish(null);
      // a start within 1.5 s of a wish change could start a plan the rider hasn't seen (§7.2 debounce)
      if (e.key !== "Enter" || !ready || !proposal || e.target instanceof HTMLButtonElement || performance.now() - wishAt.current < 1500) return;
      e.preventDefault(); // the Ready check's focused button must not receive this same Enter
      start(proposal);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [proposal, ready, wish, stop, capped]);

  const set = (patch: Partial<Checkin>) => {
    setConfirmed("changed");
    setC((prev) => ({ ...prev, ...patch }));
  };
  const compiled = proposal ? backend.compileWorkout(proposal.workout) : null;
  const cp = athleteModel[0];
  const capW = (cpW * CAP_PCT_CP.threshold) / 100;

  return (
    <div className="page today" data-pending={pending || undefined}>
      <header className="page-head">
        <h1>Tuesday</h1>
        <p className="muted">13 October</p>
      </header>
      <div className="today-grid">
        <div className="col">
          <section className="card checkin" aria-label="How are you today" data-prefilled={!confirmed || undefined}>
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
            {stop ? null : safe.checkedToday ? (
              <p className="small status-good rf-done">✓ Health check done: none of the red-flag symptoms since your last ride.</p>
            ) : (
              <RedFlagCheck compact onDone={() => undefined} />
            )}
          </section>
          {!stop && <WishCard wish={wish} outcome={outcome} capped={capped} onWish={setWish} />}
          <WeekCard />
        </div>

        <div className="col">
          {stop ? (
            <MedicalStop open={freeConfirm} onOpen={() => setFreeConfirm(true)} onClose={() => setFreeConfirm(false)} />
          ) : (
            <>
          {fx.healthCap && (
            <section className="card cap status-warning" role="status" aria-label="Intensity cap">
              <b>⚠ Capped at threshold ({CAP_PCT_CP.threshold} % of CP, {Math.round(capW)} W) · no sprints</b>
              <span className="muted"> — from your injury answer. Lift it in Settings → Health when the injury allows hard efforts.</span>
            </section>
          )}
          <section className="card proposal" aria-label="Today's proposal" aria-busy={pending} data-delivered={anyway && outcome.overridable && wish ? wish.intensity : outcome.delivered.intensity}>
            <div className="proposal-head">
              <span className="eyebrow">Coach proposes {pending && <span className="pending-dot">· re-planning…</span>}</span>
              <h2>{proposal?.workout.name ?? " "}</h2>
              <p className="big-meta">{compiled ? `${mmss(compiled.totalS)} · TSS ${Math.round(compiled.tss)} · IF ${compiled.if.toFixed(2)}` : " "}</p>
            </div>
            <div className="proposal-profile">{compiled && <Profile blocks={compiled.blocks} zones={backend.zones.power} height={100} capW={fx.healthCap ? capW : undefined} label="Proposed workout profile" />}</div>
            <WishOutcomeBlock o={outcome} anyway={anyway && outcome.overridable} onAnyway={() => setAnyway(true)} />
            <h3>Why today</h3>
            <ul className="reasons">
              {proposal &&
                proposalReasons(proposal, c.minutes, word("legs", c.legs), word("sleep", c.sleep), !!fx.healthCap && !wish).map((r) => <li key={r}>{r}</li>)}
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
            {(!confirmed || !safe.checkedToday) && <p className="small muted">Confirm how you feel and answer the health check to start.</p>}
          </section>
            </>
          )}

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

/**
 * Medical stop (DESIGN §5.8): a red-flag answer means see a doctor first. The coach proposes nothing;
 * riding is still possible only as a deliberate, logged choice without plan, targets or coach.
 */
function MedicalStop({ open, onOpen, onClose }: { open: boolean; onOpen: () => void; onClose: () => void }) {
  const [understood, setUnderstood] = useState(false);
  return (
    <section className="card medical-stop" role="alert" aria-label="See a doctor before training">
      <span className="eyebrow">No plan today</span>
      <h2>Please see a doctor before training</h2>
      <p>
        You reported a symptom that needs a medical check before exercise — chest pain or pressure, fainting, or a racing heartbeat with dizziness. crank won't plan or coach rides until you
        record a clearance. <b>If you have chest pain now, stop and call emergency services.</b>
      </p>
      <ul className="reasons">
        <li>The coach proposes nothing and stays silent.</li>
        <li>To lift this, record the clearance (date and who cleared you) in Settings → Health. It is not a one-click switch.</li>
        <li>Every check-in asks about these symptoms again.</li>
      </ul>
      {!open ? (
        <div className="actions">
          <button className="primary big" onClick={() => navigate("settings", "health")}>
            Record a medical clearance
          </button>
          <button onClick={onOpen}>Ride without a plan…</button>
        </div>
      ) : (
        <div className="free-confirm" role="dialog" aria-label="Ride without a plan">
          <p>
            <b>A free ride has no plan, no targets and no coach.</b> The trainer is set to flat-road resistance. It is your decision and it is logged with the reason.
          </p>
          <label className="check">
            <input type="checkbox" checked={understood} onChange={(e) => setUnderstood(e.target.checked)} /> I understand crank isn't advising this ride, and I'll stop at any symptom.
          </label>
          <div className="actions">
            <button autoFocus onClick={onClose}>
              Cancel
            </button>
            <button disabled={!understood} onClick={() => requestStart(freeRide(cpW), "today")}>
              Start free ride
            </button>
          </div>
        </div>
      )}
    </section>
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
  // defence in depth: the Ready check never starts a planned ride under a medical stop
  if (stopActive() && s.workout.id !== "free")
    return (
      <div className="page gate">
        <section className="card medical-stop" role="alert" aria-label="Ride blocked by the medical stop">
          <h2>Medical stop: no planned or coached ride</h2>
          <div className="actions">
            <button className="primary" autoFocus onClick={() => navigate("today")}>
              Back to Today
            </button>
          </div>
        </section>
      </div>
    );
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
    <div className="page ready saddle-screen">
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
            {!hrOk && (
              <button className="primary" autoFocus>
                Retry now
              </button>
            )}
          </li>
          {fx.healthCap && (
            <li className="status-warning">
              <span aria-hidden>⚠</span>
              <span>
                <b>Capped at threshold ({Math.round((cpW * CAP_PCT_CP.threshold) / 100)} W)</b> · no sprints (injury answer)
              </span>
            </li>
          )}
          <li className="muted">
            <span aria-hidden>i</span>
            <span>Mode ERG · the trainer holds the target; keep your cadence up</span>
          </li>
        </ul>
        {s.workout.id === "free" && (
          <p className="free-note">
            <b>Free ride</b> · no plan, no targets, coach off · your choice is logged
          </p>
        )}
        <div className="actions saddle">
          {/* the safe or usual choice is primary and focused; starting without HR is deliberate (§5.4) */}
          <button className={hrOk ? "primary" : ""} autoFocus={hrOk} disabled={!trainerOk} onClick={go}>
            {hrOk && <KeyHint a="coach" />} {hrOk ? "Start riding" : "Start without HR"}
          </button>
          <button onClick={() => navigate("today")}>
            <KeyHint a="end" /> Back
          </button>
        </div>
      </section>
    </div>
  );
}
