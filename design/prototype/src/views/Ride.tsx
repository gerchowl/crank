// Ride (DESIGN §5.2, §6). Fixed anatomy: status bar · hero · side · coach lane · road ahead.
// Phase templates change what fills the hero; states fill reserved slots. Nothing re-flows (#10).
// Every live value is its own subscription in a leaf component.
import { useEffect, useState } from "react";
import { findNode, NUDGE_TTL_S, type CoachNudge, type CompiledWorkout, type FlatBlock, type RideStatus, type StepNode, type Workout } from "@crank/shared";
import { Profile, Recovery } from "../charts";
import { blockName, blockTitle, mmss, num, signed } from "../format";
import { blockKind, cursorS, revertNudge, sensorOk, targetBand, trainerPolicy, type BlockKind } from "../mock/ext";
import { rideScreen, type SlotMetric } from "../persist";
import { backend, navigate, RECONNECT_NOTICE_S, UNDO_WINDOW_S, useMetric, usePoll, useSession, type RideSession, type SensorGap } from "../session";
import { useStore } from "../store";

const PHASE_WORD: Record<BlockKind, string> = { ramp: "Ramp", steady: "Work", onoff: "On", sprint: "Sprint", hr_recovery: "Recover", easy: "Easy", cooldown: "Cool-down" };

export function Ride() {
  const s = useSession();
  if (!s) return <p className="empty">No ride loaded. Start one from Today.</p>;
  return <RideScreen s={s} />;
}

function RideScreen({ s }: { s: RideSession }) {
  const status = useStore(s.status);
  const current = useStore(s.current);
  const nudges = useStore(s.nudges);
  const reverted = useStore(s.reverted);
  const gaps = useStore(s.gaps);
  const config = useStore(rideScreen);
  const [help, setHelp] = useState(false);
  const [confirmEnd, setConfirmEnd] = useState(false);

  const pending = nudges.find((n) => n.status === "pending");
  const undoable = status ? nudges.findLast((n) => n.status === "auto_applied" && !reverted.has(n.callId) && status.t - n.t < UNDO_WINDOW_S) : undefined;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!status || e.metaKey || e.ctrlKey || e.target instanceof HTMLInputElement) return;
      const k = e.key.toLowerCase();
      if (k === " ") {
        e.preventDefault();
        if (status.state === "riding") backend.ride.pause();
        else if (status.state === "paused") backend.ride.resume();
      } else if (k === "+" || k === "=") backend.ride.adjust(5);
      else if (k === "-") backend.ride.adjust(-5);
      else if (k === "s") backend.ride.skipBlock();
      else if (k === "y" && pending) backend.coach.respond(pending.callId, true);
      else if (k === "n" && pending) backend.coach.respond(pending.callId, false);
      else if (k === "u" && undoable) undo(s, undoable);
      else if (k === "?") setHelp((h) => !h);
      else if (k === "escape") setConfirmEnd((c) => !c);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [s, status, pending, undoable]);

  if (!status || !current) return null;
  const { workout, compiled } = current;
  const block = compiled.blocks[status.blockIndex];
  if (!block) return null;
  const step = findNode(workout.nodes, block.nodeId) as StepNode | undefined;
  const kind = blockKind(block, step, workout.ftp);
  const trainerGap = gaps.findLast((g) => g.sensor === "trainer");

  return (
    <div className="ride" data-kind={kind} data-state={status.state} data-density={config.density}>
      <StatusBar status={status} total={compiled.blocks.length} trainerGap={trainerGap} onHelp={() => setHelp((h) => !h)} />
      <section className="hero" aria-label="Now">
        <HeroHeader workout={workout} block={block} kind={kind} />
        <div className="t1-row">
          {kind === "hr_recovery" ? <ReadyIn /> : kind === "sprint" ? <Live metric="power" unit="W" className="t1" /> : <Countdown />}
        </div>
        <div className="band-row">
          <BandRow status={status} kind={kind} block={block} step={step} trainerGap={trainerGap} />
        </div>
        <Next compiled={compiled} workout={workout} index={status.blockIndex} kind={kind} />
      </section>
      <aside className="side" aria-label="Body">
        <Slot metric={config.slotA} status={status} step={step} />
        <Slot metric={config.slotB} status={status} step={step} />
        {config.density === "detail" && <Slot metric="wbal_pct" status={status} step={step} />}
        <CoachLane s={s} status={status} pending={pending} undoable={undoable} nudges={nudges} />
      </aside>
      <Road compiled={compiled} block={block} />
      {help && <KeysOverlay onClose={() => setHelp(false)} />}
      {confirmEnd && (
        <div className="dialog" role="dialog" aria-label="End ride">
          <p>End the ride now? The partial ride is kept and opens in Review.</p>
          <button className="primary" onClick={() => (backend.ride.stop(), navigate("review"))}>
            End ride
          </button>
          <button onClick={() => setConfirmEnd(false)}>Keep riding</button>
        </div>
      )}
    </div>
  );
}

function undo(s: RideSession, n: CoachNudge) {
  revertNudge(backend, n);
  s.markReverted(n.callId);
}

// ---------- status bar ----------
function StatusBar({ status, total, trainerGap, onHelp }: { status: RideStatus; total: number; trainerGap: SensorGap | undefined; onHelp: () => void }) {
  const elapsed = useMetric("elapsed", 2);
  const remaining = useMetric("remaining", 2);
  const paused = status.state === "paused";
  const back = trainerGap?.backAtT != null && status.t - trainerGap.backAtT < RECONNECT_NOTICE_S;
  return (
    <header className="statusbar">
      <button className="pause" onClick={() => (paused ? backend.ride.resume() : backend.ride.pause())} aria-label={paused ? "Resume" : "Pause"}>
        <span aria-hidden>{paused ? "▶" : "❚❚"}</span>
        <span className="lbl">{paused ? "Resume" : "Pause"}</span>
        <kbd>Space</kbd>
      </button>
      <div className="clock">
        <b>{mmss(elapsed)}</b>
        <span className="lbl">ride</span>
        <span className="sep">·</span>
        <b>{mmss(remaining)}</b>
        <span className="lbl">left</span>
      </div>
      <div className="blockno">
        <span className="lbl">block</span> <b>{status.blockIndex + 1}</b>/{total}
      </div>
      <div className="intensity" aria-label="Intensity">
        <span className="chip mode">{status.mode.toUpperCase()}</span>
        <button onClick={() => backend.ride.adjust(-5)} aria-label="Intensity −5 %">
          −
        </button>
        <b>{signed(status.manualOffsetPct)}%</b>
        <button onClick={() => backend.ride.adjust(5)} aria-label="Intensity +5 %">
          +
        </button>
      </div>
      <Sensor name="NEO" state={status.trainer} note={back && trainerGap?.backAtT ? `back · ${Math.round(trainerGap.backAtT - trainerGap.lostAtT)} s gap` : null} />
      <Sensor name="HR" state={status.hrSensor} note={null} />
      <button className="help" onClick={onHelp} aria-label="Keys">
        ?
      </button>
    </header>
  );
}

function Sensor({ name, state, note }: { name: string; state: RideStatus["trainer"]; note: string | null }) {
  const cls = sensorOk(state) ? (note ? "info" : "good") : state === "connecting" ? "warning" : "serious";
  const icon = sensorOk(state) ? (note ? "↻" : "●") : "⚠";
  const text = sensorOk(state) ? (note ?? "ok") : state === "connecting" ? "reconnecting" : "lost";
  return (
    <div className={`sensor status-${cls}`} role="status" aria-label={`${name} ${text}`}>
      <span aria-hidden>{icon}</span> <b>{name}</b> <span className="lbl">{text}</span>
    </div>
  );
}

// ---------- hero ----------
function HeroHeader({ workout, block, kind }: { workout: Workout; block: FlatBlock; kind: BlockKind }) {
  const t = blockTitle(workout.nodes, block);
  const easyRep = kind === "onoff" && block.phase !== "work";
  const reps = t.reps.at(-1);
  return (
    <div className="hero-head t3">
      <span className="phase-word">{easyRep ? "Off" : PHASE_WORD[kind]}</span>
      <span className="title">{t.label || workout.name}</span>
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
    </div>
  );
}

function Countdown({ className = "t1" }: { className?: string }) {
  const left = useMetric("block_remaining", 4);
  return (
    <div className={className} aria-label="Time left in block">
      {mmss(left)}
    </div>
  );
}

function ReadyIn() {
  const r = usePoll(1, () => backend.hrRecovery(), []);
  const ready = r.readyInS;
  return (
    <div className="t1-wrap">
      <span className="t3 lbl">ready in ≈</span>
      <span className="t1">{mmss(ready?.mean)}</span>
      <span className="t3 range">{!ready ? "fitting…" : Math.round(ready.lo) === Math.round(ready.hi) ? "" : `${mmss(ready.lo)}–${mmss(ready.hi)}`}</span>
    </div>
  );
}

function Live({ metric, unit, className }: { metric: "power" | "power_3s" | "hr" | "cadence"; unit: string; className: string }) {
  const v = useMetric(metric, 4);
  return (
    <div className={className}>
      {num(v)}
      <small>{unit}</small>
    </div>
  );
}

function BandRow({ status, kind, block, step, trainerGap }: { status: RideStatus; kind: BlockKind; block: FlatBlock; step: StepNode | undefined; trainerGap: SensorGap | undefined }) {
  if (status.state === "paused")
    return (
      <div className="band-msg">
        <b>Paused</b> · press <kbd>Space</kbd> to resume · the plan waits for you
      </div>
    );
  if (!sensorOk(status.trainer) && trainerGap)
    return (
      <div className="band-msg status-warning">
        <span aria-hidden>⚠</span> <b>NEO reconnecting · {Math.max(0, Math.round(status.t - trainerGap.lostAtT))} s</b> · {trainerPolicy(status.t - trainerGap.lostAtT)} · no power data
      </div>
    );
  if (kind === "hr_recovery") return <RecoveryRow step={step} />;
  if (kind === "sprint")
    return (
      <div className="band-split">
        <Target />
        <div className="target">
          <span className="t3 lbl">left</span>
          <Countdown className="t2" />
        </div>
      </div>
    );
  return (
    <div className="band-split">
      <Target ramp={kind === "ramp" ? block.powerTo : undefined} />
      <Gauge />
    </div>
  );
}

function Target({ ramp }: { ramp?: number | undefined }) {
  const t = useMetric("target_power", 4);
  return (
    <div className="target">
      <span className="t3 lbl">target</span>
      <span className="t2">
        {num(t)}
        <small>W</small>
      </span>
      {ramp !== undefined && <span className="t3 ramp">↗ to {Math.round(ramp)} W</span>}
    </div>
  );
}

/** Actual vs target band (P2): the marker is the 3 s power; the band and Δ come from the backend. */
function Gauge() {
  const target = useMetric("target_power", 4);
  const actual = useMetric("power_3s", 4);
  const delta = useMetric("delta_target", 4);
  if (target === null || target <= 0) return <div className="gauge" />;
  const band = targetBand(target);
  const lo = target * 0.82;
  const hi = target * 1.18;
  const u = (w: number) => `${Math.max(0, Math.min(100, ((w - lo) / (hi - lo)) * 100))}%`;
  const side = delta === null ? "none" : actual !== null && actual < band.lo ? "below" : actual !== null && actual > band.hi ? "above" : "in";
  return (
    <div className="gauge-wrap">
      <div className="gauge" data-side={side} role="meter" aria-label="Power vs target" aria-valuenow={actual ?? undefined} aria-valuemin={lo} aria-valuemax={hi}>
        <div className="gauge-band" style={{ left: u(band.lo), right: `calc(100% - ${u(band.hi)})` }} />
        {actual !== null && <div className="gauge-mark" style={{ left: u(actual) }} />}
      </div>
      <span className={`delta t3 delta-${side}`}>{delta === null ? "no data" : `${signed(delta)} W`}</span>
    </div>
  );
}

/** HR-recovery line 2: the HR target and the fitted curve (HR itself is in the side slot, §5.2). */
function RecoveryRow({ step }: { step: StepNode | undefined }) {
  const r = usePoll(1, () => backend.hrRecovery(), []);
  const target = step?.end.kind === "hr_recovery" ? step.end.targetBpm : r.targetBpm;
  return (
    <div className="band-split">
      <div className="target">
        <span className="t3 lbl">go when HR below</span>
        <span className="t2">
          {target}
          <small>bpm</small>
        </span>
      </div>
      <div className="recovery-mini">{r.observed.length > 2 ? <Recovery r={r} height={90} /> : null}</div>
    </div>
  );
}

function Next({ compiled, workout, index, kind }: { compiled: CompiledWorkout; workout: Workout; index: number; kind: BlockKind }) {
  const next = compiled.blocks[index + 1];
  if (kind === "cooldown" || !next) {
    return (
      <div className="next t3">
        <span className="lbl">next</span> ride complete — Review opens automatically
      </div>
    );
  }
  const title = blockName(workout.nodes, next);
  const w = next.powerFrom === next.powerTo ? `${Math.round(next.powerFrom)} W` : `${Math.round(next.powerFrom)}→${Math.round(next.powerTo)} W`;
  return (
    <div className="next t3">
      <span className="lbl">next</span> <b>{title}</b> {next.durRangeS ? `${mmss(next.durRangeS[0])}–${mmss(next.durRangeS[1])}` : mmss(next.durS)} @ {w}
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

function Slot({ metric, status, step }: { metric: SlotMetric; status: RideStatus; step: StepNode | undefined }) {
  const v = useMetric(metric, 2);
  const zone = useMetric("hr_zone", 1);
  const [label, unit] = SLOT_LABEL[metric];
  const lost = (metric === "hr" || metric === "dfa_a1" || metric === "hr_vs_typical") && !sensorOk(status.hrSensor);
  const sub = lost
    ? "HR strap lost"
    : metric === "hr"
      ? zone === null
        ? ""
        : `zone ${zone}`
      : metric === "cadence"
        ? step?.cadence
          ? `target ${step.cadence.lo === step.cadence.hi ? step.cadence.lo : `${step.cadence.lo}–${step.cadence.hi}`}`
          : "free"
        : "";
  const digits = metric === "dfa_a1" ? 2 : 0;
  return (
    <div className={`slot${lost ? " lost" : ""}`}>
      <span className="t3 lbl">{label}</span>
      <span className="t2">
        {lost ? "—" : metric === "hr_vs_typical" && v !== null ? signed(v) : num(v, digits)}
        <small>{unit}</small>
      </span>
      <span className={`t3 sub${lost ? " status-serious" : ""}`}>
        {lost && <span aria-hidden>⚠ </span>}
        {sub}
      </span>
    </div>
  );
}

// ---------- coach lane ----------
const VERB: Record<CoachNudge["verdict"], [string, string]> = { ease_off: ["Ease off", "Eased off"], push: ["Push", "Pushed"], hold: ["Hold", "Holding"], replan: ["Replan", "Replanned"] };
const amount = (n: CoachNudge) => (n.proposedPctChange ? ` ${signed(n.proposedPctChange)} %` : "");

function CoachLane({ s, status, pending, undoable, nudges }: { s: RideSession; status: RideStatus; pending: CoachNudge | undefined; undoable: CoachNudge | undefined; nudges: CoachNudge[] }) {
  if (pending) {
    const ttl = NUDGE_TTL_S[pending.band] || 1;
    const left = Math.max(0, pending.t + ttl - status.t);
    return (
      <div className="coach ask" role="alert">
        <div className="coach-head">
          <span className="verb">
            {VERB[pending.verdict][0]}
            {amount(pending)}?
          </span>
          <span className="band-word">{pending.band}</span>
        </div>
        <p className="t3">{pending.message}</p>
        <div className="deadline" aria-label={`${Math.round(left)} s to answer, then the plan stays`}>
          <i style={{ width: `${(left / ttl) * 100}%` }} />
        </div>
        <div className="coach-actions">
          <button className="primary" onClick={() => backend.coach.respond(pending.callId, true)}>
            <kbd>Y</kbd> Accept
          </button>
          <button onClick={() => backend.coach.respond(pending.callId, false)}>
            <kbd>N</kbd> Keep plan
          </button>
        </div>
        <span className="t3 hint">no answer in {Math.round(left)} s → plan stays</span>
      </div>
    );
  }
  if (undoable) {
    const left = Math.max(0, undoable.t + UNDO_WINDOW_S - status.t);
    return (
      <div className="coach applied" role="status">
        <div className="coach-head">
          <span className="verb">
            {VERB[undoable.verdict][1]}
            {amount(undoable)}
          </span>
          <span className="band-word">{undoable.band} · auto</span>
        </div>
        <p className="t3">{undoable.message}</p>
        <div className="deadline">
          <i style={{ width: `${(left / UNDO_WINDOW_S) * 100}%` }} />
        </div>
        <div className="coach-actions">
          <button onClick={() => undo(s, undoable)}>
            <kbd>U</kbd> Undo
          </button>
        </div>
        <span className="t3 hint">changed blocks are outlined on the road</span>
      </div>
    );
  }
  const last = nudges.findLast((n) => n.status !== "pending");
  return (
    <div className="coach idle" role="status">
      <div className="coach-head">
        <span className="verb muted">Coach is watching</span>
      </div>
      <p className="t3">
        {!sensorOk(status.hrSensor)
          ? "HR-based advice is paused while the strap is lost."
          : last
            ? `${mmss(last.t)} · ${VERB[last.verdict][0].toLowerCase()}${amount(last)} · ${last.status.replace("_", "-")}`
            : "No calls yet. It speaks when your HR, W′ or cadence say the plan should change."}
      </p>
    </div>
  );
}

// ---------- road ahead ----------
function Road({ compiled, block }: { compiled: CompiledWorkout; block: FlatBlock }) {
  const left = useMetric("block_remaining", 1);
  const now = cursorS(block, left);
  return (
    <section className="road" aria-label="Road ahead">
      <div className="road-ticks t3">
        <span style={{ left: `${(120 / 720) * 100}%` }}>now</span>
        <span className="end">+10 min</span>
      </div>
      <div className="road-near">
        <Profile blocks={compiled.blocks} zones={backend.zones.power} height={120} fromS={now - 120} toS={now + 600} cursorS={now} label="Next ten minutes" />
      </div>
      <div className="road-all">
        <Profile blocks={compiled.blocks} zones={backend.zones.power} height={28} cursorS={now} label="Whole ride" />
      </div>
    </section>
  );
}

function KeysOverlay({ onClose }: { onClose: () => void }) {
  const keys: [string, string][] = [
    ["Space", "pause / resume"],
    ["+ / −", "intensity ±5 %"],
    ["S", "skip block"],
    ["Y / N", "accept / keep plan (coach)"],
    ["U", "undo an auto-applied call"],
    ["Esc", "end ride"],
    ["?", "this list"],
  ];
  return (
    <div className="dialog keys" role="dialog" aria-label="Keys" onClick={onClose}>
      {keys.map(([k, v]) => (
        <p key={k}>
          <kbd>{k}</kbd> {v}
        </p>
      ))}
    </div>
  );
}
