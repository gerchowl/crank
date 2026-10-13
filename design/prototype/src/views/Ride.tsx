// Ride (DESIGN §5.2, §6, §7). Fixed anatomy: status bar · hero rows 1–5 · side · coach lane · road.
// Phase templates change what fills the rows; states fill reserved rows. Nothing re-flows (#10).
// Every live value is its own subscription in a leaf component.
import { Component, useEffect, useRef, useState, type ReactNode } from "react";
import { findNode, NUDGE_TTL_S, type CoachNudge, type CompiledWorkout, type FlatBlock, type RideStatus, type StepNode, type Workout } from "@crank/shared";
import { Profile, Recovery } from "../charts";
import { blockName, blockTitle, mmss, num, signed } from "../format";
import { blockKind, CAP_PCT, cadenceState, coachOffsetPct, coachReason, cursorS, recoveryRead, revertNudge, sceneFixture, settling, SETTLE_S, targetBand, type BlockKind, type CadenceState, type CoreFacts } from "../mock/ext";
import { actionFor, bindings, rideScreen, type SlotMetric } from "../persist";
import { backend, coreFacts, navigate, RECONNECT_NOTICE_S, sceneId, UNDO_WINDOW_S, useMetric, usePoll, useSession, type RideSession } from "../session";
import { play, useRideSounds } from "../sound";
import { useStore } from "../store";

const PHASE_WORD: Record<BlockKind, string> = { ramp: "Ramp", steady: "Work", onoff: "On", sprint: "Sprint", hr_recovery: "Recover", easy: "Easy", cooldown: "Cool-down" };
/** Above this width the recovery band itself becomes the T1 number (DESIGN §5.2, P7). */
const WIDE_BAND_S = 30;
/** After this long without the trainer, the banner offers fixes and End & save (§6.5). */
const LONG_LOSS_S = 30;

export function Ride() {
  const s = useSession();
  const fx = useStore(sceneFixture);
  if (!s) return <p className="empty">No ride loaded. Start one from Today.</p>;
  if (fx.screenCrashed) return <ScreenRestarted onBack={() => sceneFixture.set({})} />;
  return (
    <RideErrorBoundary>
      <RideScreen s={s} />
    </RideErrorBoundary>
  );
}

// ---------- §6.6 UI crash: the engine keeps riding ----------
/** A crash within REPEAT_WINDOW_MS of the last one counts as repeated (offers End as a second choice). */
const REPEAT_WINDOW_MS = 60_000;
const AUTO_BACK_S = 5;
class RideErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null; lastMs: number; repeated: boolean }> {
  override state = { error: null as Error | null, lastMs: -Infinity, repeated: false };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  override componentDidCatch() {
    const now = performance.now();
    this.setState((p) => ({ repeated: now - p.lastMs < REPEAT_WINDOW_MS, lastMs: now }));
  }
  override render() {
    if (!this.state.error) return this.props.children;
    return <ScreenRestarted repeated={this.state.repeated} onBack={() => this.setState({ error: null })} />;
  }
}

/** §6.6: the engine keeps riding; the default (focused, Coach button, and after 5 s by itself) is Back to ride. */
function ScreenRestarted({ onBack, repeated = false }: { onBack: () => void; repeated?: boolean }) {
  const elapsed = useMetric("elapsed", 1);
  const target = useMetric("target_power", 1);
  const [leftS, setLeftS] = useState(AUTO_BACK_S);
  useEffect(() => {
    if (sceneId) return;
    if (leftS <= 0) return onBack();
    const id = setTimeout(() => setLeftS((s) => s - 1), 1000);
    return () => clearTimeout(id);
  }, [leftS, onBack]);
  return (
    <div className="ride-restarted" role="alertdialog" aria-label="Ride screen restarted">
      <span className="eyebrow">Ride screen restarted{repeated ? " again" : ""}</span>
      <h1>Your ride kept going</h1>
      <p className="t3">
        {mmss(elapsed)} recorded · the trainer is holding {num(target)} W · nothing was lost.
      </p>
      <div className="actions">
        <button className="primary big" autoFocus onClick={onBack}>
          Back to ride · {leftS} s
        </button>
        {repeated && <button onClick={() => (backend.ride.stop(), navigate("review"))}>End ride and review</button>}
      </div>
      <p className="muted">Coach button = back to ride.</p>
    </div>
  );
}

function useNow(active: boolean) {
  const [now, setNow] = useState(() => performance.now());
  useEffect(() => {
    if (!active || sceneId) return;
    const id = setInterval(() => setNow(performance.now()), 1000);
    return () => clearInterval(id);
  }, [active]);
  return now;
}

function RideScreen({ s }: { s: RideSession }) {
  const raw = useStore(s.status);
  const current = useStore(s.current);
  const nudges = useStore(s.nudges);
  const reverted = useStore(s.reverted);
  useStore(s.gaps);
  useStore(s.pausedSinceMs);
  const fx = useStore(sceneFixture);
  const config = useStore(rideScreen);
  const keys = useStore(bindings);
  const [dialog, setDialog] = useState<"keys" | "end" | null>(null);
  const status = raw ? { ...raw, ...fx.status } : null;
  const now = useNow(status?.state === "paused" || status?.trainer !== "connected");
  const facts = status ? coreFacts(s, status, now) : null;
  const left = useMetric("block_remaining", 4);
  const feel = useFeel();

  const pending = nudges.find((n) => n.status === "pending");
  const undoable = status ? nudges.findLast((n) => n.status === "auto_applied" && !reverted.has(n.callId) && status.t - n.t < UNDO_WINDOW_S) : undefined;
  // Decision debounce (§7.2): after the lane changes (a call appears, is answered, applies), coach
  // inputs are ignored for DEBOUNCE_MS, so a press meant for the previous state can't act on the new one.
  const laneKey = pending ? `ask:${pending.callId}` : undoable ? `undo:${undoable.callId}` : fx.feelPrompt ? "feel" : "idle";
  const laneSince = useRef({ key: laneKey, at: performance.now() });
  if (laneSince.current.key !== laneKey) laneSince.current = { key: laneKey, at: performance.now() };
  useRideSounds(status, current?.compiled ?? null, nudges, left, !!facts && (facts.trainerLost || facts.hrLost || facts.ergReleased));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!status || dialog || e.metaKey || e.ctrlKey || e.repeat) return;
      // A focused control keeps its native meaning: Enter/Space on "Keep plan" keeps the plan.
      const t = e.target as HTMLElement | null;
      if (t?.closest("button, input, select, textarea, a, [role=radio], [role=switch]") && (e.key === "Enter" || e.key === " ")) return;
      if (t instanceof HTMLInputElement) return;
      if (e.key === "?") return setDialog("keys");
      if (e.key === "Escape") return setDialog("end");
      const a = actionFor(keys, e.key);
      if (!a) return;
      e.preventDefault();
      if ((a === "coach" || a === "keep") && performance.now() - laneSince.current.at < DEBOUNCE_MS) return;
      if (a === "pause") {
        if (status.state === "riding") backend.ride.pause();
        else if (status.state === "paused") backend.ride.resume();
      } else if (a === "plus") backend.ride.adjust(5);
      else if (a === "minus") backend.ride.adjust(-5);
      else if (a === "skip") backend.ride.skipBlock();
      // Coach only ever says yes; Keep is the only way back to the plan. Nothing reverses a decision
      // through the same button that made it.
      else if (a === "coach") {
        if (pending) backend.coach.respond(pending.callId, true);
        else if (fx.feelPrompt) feel.cycle();
      } else if (a === "keep") {
        if (pending) backend.coach.respond(pending.callId, false);
        else if (undoable) undo(s, undoable);
        else if (fx.feelPrompt) feel.skip();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [s, status, pending, undoable, dialog, keys, fx.feelPrompt, feel]);

  if (!status || !current || !facts) return null;
  const { workout, compiled } = current;
  const block = compiled.blocks[status.blockIndex];
  if (!block) return null;
  const step = findNode(workout.nodes, block.nodeId) as StepNode | undefined;
  const kind = blockKind(block, step, workout.ftp);
  const offset = coachOffsetPct(nudges, reverted);

  return (
    <div className="ride" data-kind={kind} data-state={facts.trainerLost ? "held" : status.state} data-density={config.density}>
      <StatusBar status={status} facts={facts} onHelp={() => setDialog("keys")} />
      <section className="hero" aria-label="Now">
        <HeroHeader workout={workout} block={block} kind={kind} step={step} capped={!!fx.healthCap} showCadence={config.density === "focus" || (config.slotA !== "cadence" && config.slotB !== "cadence")} />
        <Rows status={status} facts={facts} kind={kind} block={block} prev={compiled.blocks[status.blockIndex - 1]} step={step} offset={offset} left={left} />
        <Next compiled={compiled} workout={workout} index={status.blockIndex} kind={kind} />
      </section>
      <aside className="side" aria-label="Body">
        {config.density !== "focus" && (
          <>
            <Slot metric={config.slotA} status={status} step={step} />
            <Slot metric={config.slotB} status={status} step={step} />
          </>
        )}
        <CoachLane s={s} status={status} facts={facts} pending={pending} undoable={undoable} nudges={nudges} feel={fx.feelPrompt ? feel : null} detail={config.density === "detail"} />
      </aside>
      <Road compiled={compiled} block={block} status={status} pending={pending} />
      {dialog === "keys" && (
        <Dialog label="Keys" onClose={() => setDialog(null)}>
          <KeysList />
        </Dialog>
      )}
      {dialog === "end" && (
        <Dialog label="End ride" onClose={() => setDialog(null)}>
          <p>End the ride now? The partial ride is kept and opens in Review.</p>
          {/* the safe choice takes focus first: a Pause/Space press must never end the ride */}
          <button onClick={() => setDialog(null)}>Keep riding</button>
          <button className="primary" onClick={() => (backend.ride.stop(), navigate("review"))}>
            End ride
          </button>
        </Dialog>
      )}
    </div>
  );
}

const DEBOUNCE_MS = 1500;
const FEEL_COMMIT_MS = 3000;
const FEEL_WORDS = ["easy", "right", "hard"] as const;
type Feel = { choice: (typeof FEEL_WORDS)[number] | null; committed: boolean; cycle: () => void; skip: () => void };
/** "How did that feel?": Coach cycles easy → right → hard; the choice commits 3 s after the last press (tone); Keep skips. */
function useFeel(): Feel {
  const [choice, setChoice] = useState<Feel["choice"]>(null);
  const [committed, setCommitted] = useState(false);
  useEffect(() => {
    if (!choice || committed || sceneId) return;
    const id = setTimeout(() => (setCommitted(true), play.applied()), FEEL_COMMIT_MS);
    return () => clearTimeout(id);
  }, [choice, committed]);
  return {
    choice,
    committed,
    cycle: () => !committed && setChoice((c) => FEEL_WORDS[(c ? FEEL_WORDS.indexOf(c) + 1 : 0) % FEEL_WORDS.length] ?? "easy"),
    skip: () => (setChoice(null), setCommitted(true)),
  };
}

function undo(s: RideSession, n: CoachNudge) {
  revertNudge(backend, n);
  s.markReverted(n.callId);
  play.undone();
}

/** Modal dialog: takes focus, traps Tab, closes on Escape; ride shortcuts are suspended while open (§6.7). */
function Dialog({ label, onClose, children }: { label: string; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    const focusables = () => [...(el?.querySelectorAll<HTMLElement>("button, [href], input, select, textarea") ?? [])];
    (focusables()[0] ?? el)?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // consume it: the ride's own Escape (= open End ride) must not see the key that closed this
        e.preventDefault();
        e.stopImmediatePropagation();
        onClose();
      } else if (e.key === "Tab") {
        const f = focusables();
        e.preventDefault();
        if (!f.length) return;
        const i = f.indexOf(document.activeElement as HTMLElement);
        const next = e.shiftKey ? (i <= 0 ? f.length - 1 : i - 1) : i === f.length - 1 ? 0 : i + 1;
        f[next]?.focus();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);
  return (
    <div className="scrim">
      <div ref={ref} className="dialog" role="dialog" aria-modal="true" aria-label={label} tabIndex={-1}>
        {children}
      </div>
    </div>
  );
}

// ---------- status bar ----------
function StatusBar({ status, facts, onHelp }: { status: RideStatus; facts: CoreFacts; onHelp: () => void }) {
  const remaining = useMetric("remaining", 2);
  const paused = status.state === "paused";
  const back = facts.sinceReconnectS !== null && facts.sinceReconnectS < SETTLE_S + RECONNECT_NOTICE_S && facts.lastGapS !== null;
  return (
    <header className="statusbar">
      <button className="pause" onClick={() => (paused ? backend.ride.resume() : backend.ride.pause())}>
        <span aria-hidden>{paused ? "▶" : "❚❚"}</span>
        <span>{paused ? "Resume" : "Pause"}</span>
      </button>
      <div className="clock t3">
        <b>{mmss(remaining)}</b> <span className="lbl">left</span>
      </div>
      <div className="intensity t3" aria-label="Your intensity offset">
        <span className="chip mode">{status.mode.toUpperCase()}</span>
        <span className="lbl you">you</span>
        <button onClick={() => backend.ride.adjust(-5)} aria-label="Intensity −5 %">
          −
        </button>
        <b>{signed(status.manualOffsetPct)}%</b>
        <button onClick={() => backend.ride.adjust(5)} aria-label="Intensity +5 %">
          +
        </button>
      </div>
      <Sensor name="NEO" state={facts.trainerLost ? "lost" : status.trainer} note={back ? `back · ${mmss(facts.lastGapS)} gap` : null} />
      <Sensor name="HR" state={status.hrSensor} note={null} />
      <button className="help" onClick={onHelp} aria-label="Keys">
        ?
      </button>
    </header>
  );
}

function Sensor({ name, state, note }: { name: string; state: RideStatus["trainer"]; note: string | null }) {
  const ok = state === "connected";
  const cls = ok ? (note ? "info" : "good") : state === "connecting" ? "warning" : "serious";
  const text = ok ? (note ?? "") : state === "connecting" ? "…" : "lost";
  return (
    <div className={`sensor status-${cls}`} role="status" aria-label={`${name} ${ok ? (note ?? "connected") : text}`}>
      <span aria-hidden>{ok ? (note ? "↻" : "●") : "⚠"}</span> <b>{name}</b> <span className="lbl">{text}</span>
    </div>
  );
}

// ---------- hero ----------
function HeroHeader({ workout, block, kind, step, showCadence, capped }: { workout: Workout; block: FlatBlock; kind: BlockKind; step: StepNode | undefined; showCadence: boolean; capped: boolean }) {
  const t = blockTitle(workout.nodes, block);
  const inherited = !step?.label && block.phase !== "work" && kind !== "hr_recovery";
  const easyRep = kind === "onoff" && block.phase !== "work";
  const reps = t.reps.at(-1);
  const cadence = useMetric("cadence", 2);
  const cs = cadenceState(cadence, step);
  return (
    <div className="row1 t3" data-fit>
      <span className="phase-word">{easyRep ? "Off" : PHASE_WORD[kind]}</span>
      <span className="title">{t.label ? (inherited ? `after ${t.label}` : t.label) : workout.name}</span>
      {t.reps.map((r, i) => (
        <span key={i} className="rep">
          {i === t.reps.length - 1 ? "rep" : "set"} {r.i}/{r.n}
        </span>
      ))}
      {kind === "onoff" && reps && (
        <span className="pips" aria-hidden>
          {Array.from({ length: reps.n }, (_, i) => (
            <i key={i} className={i < reps.i - 1 ? "done" : i === reps.i - 1 ? "now" : ""} />
          ))}
        </span>
      )}
      {showCadence && cs !== "in" && cs !== "none" && <span className={`chip cad-${cs}`}>{CADENCE_WORD[cs]}</span>}
      {capped && <span className="chip cap-chip">capped ≤ {CAP_PCT.endurance} % FTP</span>}
    </div>
  );
}

interface RowProps { status: RideStatus; facts: CoreFacts; kind: BlockKind; block: FlatBlock; prev: FlatBlock | undefined; step: StepNode | undefined; offset: number; left: number | null }

function Rows(props: RowProps) {
  const { kind, facts } = props;
  const rec = usePoll(1, () => (kind === "hr_recovery" ? backend.hrRecovery() : null), [kind, props.block.nodeId]);
  const read = kind === "hr_recovery" && rec ? recoveryRead(rec, props.status.t - rec.startedT, facts.hrLost) : null;
  // The caption says which event the number is about: HR crossing (a prediction) or the block's own limit (a rule).
  const caption =
    kind === "sprint"
      ? "power"
      : read
        ? read.kind === "hr_lost"
          ? "HR lost · block ends at its maximum in"
          : read.kind === "ends_at_min"
            ? "block ends at its minimum in"
            : read.kind === "estimate" && read.capped
              ? `HR below ${rec?.targetBpm} in · or at the block maximum`
              : `HR below ${rec?.targetBpm} in`
        : kind === "onoff"
          ? "left in rep"
          : "left in block";
  let t1: ReactNode;
  if (kind === "sprint") t1 = <Live metric="power" unit="W" />;
  else if (read) {
    if (read.kind === "hr_lost" || read.kind === "ends_at_min") t1 = <div className="t1">{mmss(read.inS)}</div>;
    else if (read.kind === "fitting") t1 = <div className="t1 muted">fitting…</div>;
    else if (read.hi - read.lo > WIDE_BAND_S)
      t1 = (
        <div className="t1 range-mode">
          {mmss(read.lo)}–{mmss(read.hi)}
        </div>
      );
    else
      t1 = (
        <div className="t1-wrap">
          <span className="t1">≈ {mmss(read.mean)}</span>
          <span className="t3 range">
            {mmss(read.lo)}–{mmss(read.hi)}
          </span>
        </div>
      );
  } else t1 = <div className="t1">{mmss(props.left)}</div>;
  return (
    <>
      <div className="row2 t3 lbl" data-fit>
        {caption}
      </div>
      <div className="row3">{t1}</div>
      <div className="row4">
        <BandRow {...props} rec={rec} />
      </div>
    </>
  );
}

function Live({ metric, unit }: { metric: "power" | "power_3s"; unit: string }) {
  const v = useMetric(metric, 4);
  return (
    <div className="t1">
      {num(v)}
      <small>{unit}</small>
    </div>
  );
}

function Banner({ tone, children }: { tone: "warning" | "info" | "neutral"; children: ReactNode }) {
  return (
    <div className={`band-msg tone-${tone}`} role={tone === "warning" ? "alert" : "status"} data-fit>
      {children}
    </div>
  );
}

function BandRow({ status, facts, kind, block, prev, step, offset, left, rec }: RowProps & { rec: ReturnType<typeof backend.hrRecovery> | null }) {
  const target = useMetric("target_power", 2);
  const cadence = useMetric("cadence", 2);
  if (facts.trainerLost) {
    const at = mmss(kind === "hr_recovery" && rec ? status.t - rec.startedT : block.durS - (left ?? 0));
    const long = facts.lostForS >= LONG_LOSS_S;
    return (
      <Banner tone="warning">
        <b>
          ⚠ {status.hrSensor !== "connected" ? "Trainer + HR lost" : "Trainer lost"} · paused at {at}
        </b>
        {long ? (
          <>
            <span>try: power-cycle the NEO · close Zwift/Tacx</span>
            <span className="banner-actions">
              <button onClick={() => (backend.ride.stop(), navigate("review"))}>End & save</button>
              <span>retry {facts.reconnectAttempt}</span>
            </span>
          </>
        ) : (
          <span>reconnecting (try {facts.reconnectAttempt}) · ride saved</span>
        )}
      </Banner>
    );
  }
  if (facts.ergReleased)
    return (
      <Banner tone="warning">
        <b>⚠ ERG released · spin up to 60 rpm</b>
        <span>timer runs · target returns over 5 s</span>
      </Banner>
    );
  if (facts.sinceReconnectS !== null && facts.sinceReconnectS < SETTLE_S)
    return (
      <Banner tone="info">
        <b>↻ Reconnected · easing in to {num(target)} W</b>
        <span>{(cadence ?? 0) > 0 ? "workout resumed" : "pedal to resume the workout"}</span>
      </Banner>
    );
  if (status.state === "paused")
    return (
      <Banner tone="neutral">
        <b>Paused {mmss(facts.pausedForS ?? 0)}</b>
        <span>Resume continues the plan · trainer free</span>
      </Banner>
    );
  if (kind === "hr_recovery" && rec) return <RecoveryRow rec={rec} step={step} lost={facts.hrLost} status={status} />;
  if (kind === "sprint") return <Fill target={target} left={left} />;
  const sub = offset && block.aiChanged ? `coach ${signed(offset)} %` : kind === "ramp" ? `↗ to ${Math.round(block.powerTo)} W` : "";
  const blockElapsed = block.durS - (left ?? block.durS);
  const settle = settling(block, prev, blockElapsed, facts.sinceReconnectS);
  return (
    <div className="band-split">
      <div className="target">
        <span className="t3 lbl">target</span>
        <span className="t2">
          {num(target)}
          <small>W</small>
        </span>
        <span className="t3 sub" data-fit>
          {sub}
        </span>
      </div>
      <Gauge target={target} settle={settle} />
    </div>
  );
}

/** Actual vs target band (P2). Hidden while the trainer settles (§5.2). */
function Gauge({ target, settle }: { target: number | null; settle: boolean }) {
  const actual = useMetric("power_3s", 4);
  const delta = useMetric("delta_target", 4);
  if (target === null || target <= 0) return <div className="gauge-wrap" />;
  const band = targetBand(target);
  const lo = target * 0.82;
  const hi = target * 1.18;
  const u = (w: number) => `${Math.max(0, Math.min(100, ((w - lo) / (hi - lo)) * 100))}%`;
  const side = settle || delta === null || actual === null ? "none" : actual < band.lo ? "below" : actual > band.hi ? "above" : "in";
  return (
    <div className="gauge-wrap">
      <div className="gauge" data-side={side} role="meter" aria-label="Power vs target band" aria-valuenow={actual ?? undefined} aria-valuemin={lo} aria-valuemax={hi}>
        <div className="gauge-band" style={{ left: u(band.lo), right: `calc(100% - ${u(band.hi)})` }} />
        {!settle && actual !== null && <div className="gauge-mark" style={{ left: u(actual) }} />}
      </div>
      <span className={`delta t3 delta-${side}`}>
        {settle ? "settling" : delta === null ? "no data" : side === "in" ? "on target" : `${side === "below" ? "▼" : "▲"} ${Math.abs(Math.round(delta))} W`}
      </span>
    </div>
  );
}

/** Sprint: actual power as a fill against the target tick (no second number to compare) + countdown. */
function Fill({ target, left }: { target: number | null; left: number | null }) {
  const actual = useMetric("power", 4);
  const pct = target && actual !== null ? Math.max(0, Math.min(1.3, actual / target)) : 0;
  return (
    <div className="sprint-row">
      <div className="fill-wrap">
        <span className="t3 lbl" data-fit>
          target {num(target)} W (the line)
        </span>
        <div className="fill" role="meter" aria-label="Power vs sprint target" aria-valuenow={actual ?? undefined}>
          <i style={{ width: `${(pct / 1.3) * 100}%` }} />
          <b style={{ left: `${(1 / 1.3) * 100}%` }} />
        </div>
      </div>
      <div className="target">
        <span className="t3 lbl">left</span>
        <span className="t2">{mmss(left)}</span>
      </div>
    </div>
  );
}

function RecoveryRow({ rec, step, lost, status }: { rec: ReturnType<typeof backend.hrRecovery>; step: StepNode | undefined; lost: boolean; status: RideStatus }) {
  const hr = useMetric("hr", 2);
  const end = step?.end.kind === "hr_recovery" ? step.end : null;
  const toGo = hr !== null ? Math.max(0, hr - rec.targetBpm) : null;
  const elapsed = status.t - rec.startedT;
  return (
    <div className="band-split">
      <div className="target">
        <span className="t3 lbl">to go</span>
        <span className="t2">
          {lost ? "—" : num(toGo)}
          <small>bpm</small>
        </span>
        <span className="t3 sub" data-fit>
          {end ? (Math.max(0, end.minS - elapsed) > 0 && (rec.readyInS?.hi ?? Infinity) <= end.minS - elapsed + 1 ? `HR due below ${rec.targetBpm} before then` : `block ends in ${mmss(Math.max(0, end.minS - elapsed))}–${mmss(Math.max(0, end.maxS - elapsed))}`) : ""}
        </span>
      </div>
      <div className="recovery-mini">{rec.observed.length > 2 ? <Recovery r={rec} height={84} live={!lost && status.state === "riding"} predict={!lost} /> : null}</div>
    </div>
  );
}

function Next({ compiled, workout, index, kind }: { compiled: CompiledWorkout; workout: Workout; index: number; kind: BlockKind }) {
  const next = compiled.blocks[index + 1];
  if (kind === "cooldown" || !next) {
    return (
      <div className="row5 t3p" data-fit>
        <span className="lbl">next</span> ride complete → Review
      </div>
    );
  }
  const w = next.powerFrom === next.powerTo ? `${Math.round(next.powerFrom)} W` : `${Math.round(next.powerFrom)}→${Math.round(next.powerTo)} W`;
  return (
    <div className="row5 t3p" data-fit>
      <span className="lbl">next</span> <b>{blockName(workout.nodes, next)}</b> {next.durRangeS ? `${mmss(next.durRangeS[0])}–${mmss(next.durRangeS[1])}` : mmss(next.durS)} @ {w}
    </div>
  );
}

// ---------- side ----------
const SLOT_LABEL: Record<SlotMetric, [string, string]> = {
  hr: ["Heart rate", "bpm"],
  cadence: ["Cadence", "rpm"],
  wbal_pct: ["W′ balance", "%"],
  dfa_a1: ["DFA α1", ""],
  hr_vs_typical: ["HR vs usual", "bpm"],
  power_10s: ["Power 10 s", "W"],
  kj: ["Work", "kJ"],
};
const CADENCE_WORD: Record<CadenceState, string> = { in: "✓ in range", below: "▲ spin faster", above: "▼ ease cadence", none: "no target" };

function Slot({ metric, status, step }: { metric: SlotMetric; status: RideStatus; step: StepNode | undefined }) {
  const v = useMetric(metric, 2);
  const zone = useMetric("hr_zone", 1);
  const [label, unit] = SLOT_LABEL[metric];
  const hrBased = metric === "hr" || metric === "dfa_a1" || metric === "hr_vs_typical";
  const lost = (hrBased && status.hrSensor !== "connected") || (!hrBased && metric !== "kj" && status.trainer !== "connected");
  const cs = metric === "cadence" ? cadenceState(v, step) : null;
  const sub = lost ? `${hrBased ? "HR strap" : "trainer"} lost` : metric === "hr" ? (zone === null ? "" : `zone ${zone}`) : cs ? CADENCE_WORD[cs] : "";
  return (
    <div className={`slot${lost ? " lost" : ""}`}>
      <span className="t3 lbl">{label}</span>
      <span className={`t3 sub${lost ? " status-serious" : cs ? ` cad-${cs}` : ""}`} data-fit>
        {lost && <span aria-hidden>⚠ </span>}
        {sub}
      </span>
      <span className="t2 val">
        {lost ? "—" : metric === "hr_vs_typical" && v !== null ? signed(v) : num(v, metric === "dfa_a1" ? 2 : 0)}
        <small>{unit}</small>
      </span>
      {cs && step?.cadence && !lost && v !== null ? <CadenceBand rpm={v} lo={step.cadence.lo} hi={step.cadence.hi} /> : <div className="cad-band reserved" aria-hidden />}
    </div>
  );
}

function CadenceBand({ rpm, lo, hi }: { rpm: number; lo: number; hi: number }) {
  const a = lo - 20;
  const b = hi + 20;
  const u = (x: number) => `${Math.max(0, Math.min(100, ((x - a) / (b - a)) * 100))}%`;
  return (
    <div className="cad-band" aria-hidden>
      <div className="gauge-band" style={{ left: u(lo), right: `calc(100% - ${u(hi)})` }} />
      <div className="gauge-mark" style={{ left: u(rpm) }} />
    </div>
  );
}

// ---------- coach lane ----------
const VERB: Record<CoachNudge["verdict"], [string, string]> = { ease_off: ["Ease off", "Eased off"], push: ["Push", "Pushed"], hold: ["Hold", "Holding"], replan: ["Replan", "Replanned"] };
const amount = (n: CoachNudge) => (n.proposedPctChange ? ` ${signed(n.proposedPctChange)} %` : "");

function CoachLane({ s, status, facts, pending, undoable, nudges, feel, detail }: { s: RideSession; status: RideStatus; facts: CoreFacts; pending: CoachNudge | undefined; undoable: CoachNudge | undefined; nudges: CoachNudge[]; feel: Feel | null; detail: boolean }) {
  const wbal = useMetric("wbal_pct", 1);
  if (feel && !feel.committed)
    return (
      <div className="coach ask" role="alert">
        <div className="coach-head">
          <span className="verb" data-fit>
            How was that?
          </span>
          <span className="band-word">calibrating</span>
        </div>
        <div className="coach-actions three" role="radiogroup" aria-label="How did that feel">
          {FEEL_WORDS.map((w) => (
            <button key={w} role="radio" aria-checked={feel.choice === w} className={feel.choice === w ? "on" : ""} onClick={() => feel.cycle()}>
              {w}
            </button>
          ))}
        </div>
        <span className="t3 deadline-text" data-fit>
          {feel.choice ? `saves “${feel.choice}” in 3 s` : "Coach picks · none in 12 s"}
        </span>
      </div>
    );
  if (pending) {
    const ttl = NUDGE_TTL_S[pending.band] || 1;
    const leftS = Math.max(0, pending.t + ttl - status.t);
    return (
      <div className="coach ask" role="alert">
        <div className="coach-head">
          <span className="verb" data-fit>
            {VERB[pending.verdict][0]}
            {amount(pending)}?
          </span>
          <span className="band-word">{pending.band}</span>
        </div>
        <p className="t3 reason" data-fit>
          {coachReason(pending)}
        </p>
        <div className="coach-actions">
          <button onClick={() => backend.coach.respond(pending.callId, true)}>Accept</button>
          <button onClick={() => backend.coach.respond(pending.callId, false)}>Keep plan</button>
        </div>
        <span className="t3 deadline-text" data-fit>
          keeps plan in {Math.ceil(leftS)} s
        </span>
      </div>
    );
  }
  if (undoable) {
    const leftS = Math.max(0, undoable.t + UNDO_WINDOW_S - status.t);
    return (
      <div className="coach applied" role="status">
        <div className="coach-head">
          <span className="verb" data-fit>
            {VERB[undoable.verdict][1]}
            {amount(undoable)}
          </span>
          <span className="band-word">auto</span>
        </div>
        <p className="t3 reason" data-fit>
          {coachReason(undoable)}
        </p>
        <div className="coach-actions">
          <button onClick={() => undo(s, undoable)}>Back to plan · {Math.ceil(leftS)} s</button>
        </div>
        <span className="t3 deadline-text" data-fit>
          Keep = back to plan
        </span>
      </div>
    );
  }
  const last = nudges.findLast((n) => n.status !== "pending");
  return (
    <div className="coach idle" role="status">
      {/* Detail density puts its third metric here, in the reserved lane a coach call takes over (§5.3) */}
      {detail ? (
        <div className="idle-metric detail">
          <span className="t3 lbl">W′ balance</span>
          <span className="t2">{facts.trainerLost ? <span className="stale">—</span> : <>{num(wbal)}<small>%</small></>}</span>
        </div>
      ) : (
        <div className="idle-metric t3" data-fit>
          <span className="lbl">W′ balance</span> {facts.trainerLost ? <span className="stale">— no power data</span> : <b>{num(wbal)} %</b>}
        </div>
      )}
      <p className="t3 idle-note" data-fit>
        {facts.hrLost ? "HR lost: coach can't push" : last ? `last call ${mmss(last.t)}${amount(last)} ${s.reverted.get().has(last.callId) ? "back to plan" : last.status === "auto_applied" || last.status === "accepted" ? "applied" : "not taken"}` : "coach: no calls yet"}
      </p>
    </div>
  );
}

// ---------- road ahead ----------
function Road({ compiled, block, status, pending }: { compiled: CompiledWorkout; block: FlatBlock; status: RideStatus; pending: CoachNudge | undefined }) {
  const affected = pending ? compiled.blocks.filter((b, i) => i >= status.blockIndex && b.phase === "work").length : 0;
  const left = useMetric("block_remaining", 1);
  const elapsed = useMetric("elapsed", 1);
  const now = cursorS(block, left);
  return (
    <section className="road" aria-label="Road ahead">
      <div className="road-ticks">
        <span style={{ left: `${(120 / 720) * 100}%` }}>now</span>
        <span className="legend">{pending?.proposedPctChange ? `dashed = proposed ${signed(pending.proposedPctChange)} % on ${affected} work block${affected === 1 ? "" : "s"}` : "coach changes: solid = applied"}</span>
        <span className="end">+10 min</span>
      </div>
      <div className="road-near">
        <Profile blocks={compiled.blocks} zones={backend.zones.power} height={120} fromS={now - 120} toS={now + 600} cursorS={now} pending={pending?.proposedPctChange ? { from: status.blockIndex, pct: pending.proposedPctChange } : undefined} label="Next ten minutes" />
      </div>
      <div className="road-all">
        <Profile blocks={compiled.blocks} zones={backend.zones.power} height={24} cursorS={now} label="Whole ride" />
        <span className="road-meta">
          {mmss(elapsed)} ridden · block {status.blockIndex + 1} of {compiled.blocks.length}
        </span>
      </div>
    </section>
  );
}

function KeysList() {
  const b = useStore(bindings);
  const show = (k: string) => (k === " " ? "Space" : k);
  const rows: [keyof typeof b, string][] = [
    ["coach", "Coach: accept a question · choose in the feel prompt (never undoes)"],
    ["keep", "Keep: keep the plan · back to plan after an auto change"],
    ["pause", "Pause / resume"],
    ["skip", "Skip block"],
    ["minus", "Intensity −5 %"],
    ["plus", "Intensity +5 %"],
  ];
  return (
    <>
      {rows.map(([a, label]) => (
        <p key={a}>
          {b[a].map((k) => (
            <kbd key={k}>{show(k)}</kbd>
          ))}{" "}
          {label}
        </p>
      ))}
      <p>
        <kbd>Esc</kbd> end ride · <kbd>?</kbd> this list · remote buttons: Settings → Controls
      </p>
    </>
  );
}
