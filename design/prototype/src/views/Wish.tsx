// The planner wish (DESIGN §15.2): the rider points into intensity × format; the planner answers with what
// it proposes and, where that differs, why. The matrix shows both marks, so a deviation is a distance on the
// grid (P2). After the check-in the proposal becomes a saddle card (wish, session, one reason, actions at T3,
// above the fold); a remote reaches every cell through the wish mode. Safety is never one of the choices.
import { useStore } from "../store";
import { mmss } from "../format";
import { bindings, type Action } from "../persist";
import { cellName, coherent, FORMAT_NAME, FORMATS, fmtMin, INTENSITIES, INTENSITY_NAME, lockedByCap, sameCell, type Cell, type Wish, type WishOutcome, type WishRecord } from "../mock/wish";
import { WhyChip } from "./Why";

export function Key({ a }: { a: Action }) {
  const k = useStore(bindings)[a][0] ?? "";
  return <kbd>{k === "Backspace" ? "⌫" : k === " " ? "Space" : k}</kbd>;
}

/** The grid: plain buttons in reading order (Tab / arrow-free), each announcing its state in words. */
function Matrix({ o, capped, cursor, onPick, provisional = false }: { o: WishOutcome; capped: boolean; cursor?: Cell; onPick?: (c: Cell) => void; provisional?: boolean }) {
  const adapted = o.relation === "adapted";
  return (
    <div className={cursor ? "matrix big" : "matrix"} role="group" aria-label="Wish: main effort × format">
      <span />
      {FORMATS.map((f) => (
        <span key={f} className="axis">
          {FORMAT_NAME[f]}
        </span>
      ))}
      {[...INTENSITIES].reverse().map((i) => (
        <div key={i} className="matrix-row">
          <span className="axis row-axis">
            {INTENSITY_NAME[i][0]}
            <small>{INTENSITY_NAME[i][1]}</small>
          </span>
          {FORMATS.map((f) => {
            const c: Cell = { intensity: i, format: f };
            if (!coherent(c)) return <span key={f} className="cell none" aria-hidden />;
            const locked = lockedByCap(c, capped);
            const isWish = sameCell(o.wish, c);
            const isPlan = sameCell(o.delivered, c);
            const word = locked ? "cap" : isWish && isPlan ? (adapted ? "≈ wish" : provisional ? "↻ wish" : "✓ wish") : isWish ? "wish" : isPlan ? (o.wish ? "plan" : "coach") : "";
            return (
              <button
                key={f}
                className={`cell${isWish ? " wished" : ""}${isPlan ? " planned" : ""}${adapted && isPlan ? " adapted" : ""}${locked ? " locked" : ""}${sameCell(cursor, c) ? " cursor" : ""}`}
                aria-pressed={isWish}
                aria-label={`${cellName(c)}${word ? `: ${word.replace("✓", "granted").replace("≈", "adapted")}` : ""}${locked ? " — above your cap" : ""}`}
                disabled={locked || !onPick}
                data-cell={`${i}.${f}`}
                onClick={() => onPick?.(c)}
              >
                {word}
              </button>
            );
          })}
        </div>
      ))}
    </div>
  );
}

/** Desk: the full matrix, one click per wish; a carried-over wish is shown with its expiry. */
/** `provisional`: the check-in isn't confirmed yet, so nothing reads as granted (↻, not ✓). */
export function WishCard({ o, capped, carried, provisional, onWish, onDropCarried }: { o: WishOutcome; capped: boolean; carried: WishRecord | null; provisional: boolean; onWish: (w: Wish) => void; onDropCarried: () => void }) {
  return (
    <section className="card wish" aria-label="Your wish">
      <h2>Anything you'd like to ride?</h2>
      <p className="wish-now" data-fit>
        {o.wish ? <>Wish: <b>{cellName(o.wish)}</b></> : <>No wish — <b>the coach picks</b></>}
      </p>
      {carried && (
        <p className="carried small" role="status">
          ↻ Carried over from {carried.setOn}: <b>{cellName(carried.cell)}</b> · expires {carried.expires}. <button onClick={onDropCarried}>Drop it</button>
        </p>
      )}
      <Matrix o={o} capped={capped} provisional={provisional} onPick={(c) => onWish(sameCell(o.wish, c) ? null : c)} />
      <p className="small muted">
        {capped ? "Your injury cap: nothing above threshold, ERG only — no hills in SIM, open rides or race surges. " : ""}How long is set by Time above. From the saddle: <Key a="skip" /> wish mode.
      </p>
      {o.wish && <button onClick={() => onWish(null)}>Let the coach pick</button>}
    </section>
  );
}

export function outcomeHead(o: WishOutcome, anyway: boolean, askedMin: number, provisional = false): string {
  const w = o.wish ?? o.delivered;
  if (anyway) return `Your call: ${cellName(w)} · logged`;
  if (provisional && o.relation === "as_wished") return `↻ Your wish, after your check-in: ${cellName(w)}`;
  const steps = INTENSITIES.indexOf(w.intensity) - INTENSITIES.indexOf(o.delivered.intensity);
  // one line at T3+ (≤ ~36 characters): the format is named once when it didn't change
  const change = w.format === o.delivered.format ? `${INTENSITY_NAME[w.intensity][0]} → ${cellName(o.delivered)}` : `${cellName(w)} → ${cellName(o.delivered)}`;
  switch (o.relation) {
    case "no_wish":
      return `Coach's pick: ${cellName(o.delivered)}`;
    case "as_wished":
      return `✓ Your wish: ${cellName(o.delivered)}`;
    case "adapted":
      return `≈ Wish, shortened: ${fmtMin(o.minutes)}, not ${fmtMin(askedMin)}`;
    case "deferred":
      return `Wish ${change}, ${steps} step${steps === 1 ? "" : "s"} easier`;
    case "blocked_by_safety":
      return `⚠ Cap: ${change}`;
  }
}

/** After the check-in: the proposal read from the bike — the wish, the session, one reason, the actions (§15.2). */
export function SaddleProposal(p: {
  o: WishOutcome;
  anyway: boolean;
  askedMin: number;
  name: string;
  totalS: number;
  fallbackReason: string;
  ready: boolean;
  onStart: () => void;
  onAnyway: () => void;
  onRemote: () => void;
  onStep: (dir: -1 | 1) => void;
  children?: React.ReactNode;
}) {
  const r = p.anyway ? undefined : p.o.reasons[0];
  return (
    <section className="card saddle-proposal" data-saddle aria-label="Today's proposal">
      <p className={`outcome-line t3p rel-${p.anyway ? "anyway" : p.o.relation}`} data-above-fold data-tier="t3plus" data-fit aria-live="polite" data-relation={p.anyway ? "anyway" : p.o.relation}>
        {outcomeHead(p.o, p.anyway, p.askedMin)}
      </p>
      <h2 className="t3p" data-above-fold data-tier="t3plus" data-fit>
        {p.name} · {mmss(p.totalS)}
      </h2>
      <p className="t3 reason-line" data-above-fold data-tier="t3">
        {r ? r.text : p.fallbackReason} {r?.rule && <WhyChip id={r.rule} reason={r} />}
        {!p.anyway && p.o.deferredTo && <span className="muted"> Your wish moves to {p.o.deferredTo}.</span>}
      </p>
      {p.children}
      <div className="actions saddle" data-above-fold>
        <button className="primary" disabled={!p.ready} onClick={p.onStart}>
          <Key a="coach" /> Start
        </button>
        {p.o.overridable && !p.anyway && (
          <button disabled={!p.ready} onClick={p.onAnyway}>
            {p.o.relation === "adapted" ? `Ride the full ${fmtMin(p.askedMin)}` : "Ride my wish"}
          </button>
        )}
        <button onClick={p.onRemote}>
          <Key a="skip" /> Other wish
        </button>
        <button onClick={() => p.onStep(-1)}>
          <Key a="minus" /> Easier
        </button>
        <button onClick={() => p.onStep(1)}>
          <Key a="plus" /> Harder
        </button>
      </div>
    </section>
  );
}

/** Remote wish mode: both axes from the bars; nothing applies until Coach, Keep cancels. */
export function RemoteWish({ o, cursor, capped, onUse, onCancel }: { o: WishOutcome; cursor: Cell; capped: boolean; onUse: () => void; onCancel: () => void }) {
  return (
    <div className="remote-wish" role="dialog" aria-label="Choose a wish">
      {/* focus on the dialog, not a button: Space (Pause = next) must step, not press "Use" (§7.2 focused controls win) */}
      <section className="card saddle-screen" tabIndex={-1} ref={(el) => el?.focus()}>
        <span className="eyebrow">Wish mode · nothing changes until you use it</span>
        <p className="t2-name" aria-live="polite" data-fit>
          {cellName(cursor)}
        </p>
        <Matrix o={o} capped={capped} cursor={cursor} />
        <p className="t3 wish-grammar">
          <Key a="minus" /> <Key a="plus" /> effort · <Key a="skip" /> format · <Key a="pause" /> next
        </p>
        <div className="actions saddle">
          <button className="primary" onClick={onUse}>
            <Key a="coach" /> Use this wish
          </button>
          <button onClick={onCancel}>
            <Key a="keep" /> Cancel
          </button>
        </div>
      </section>
    </div>
  );
}
