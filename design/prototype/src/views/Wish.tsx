// The planner wish (DESIGN §15.2): the rider points into intensity × format; the planner answers with what
// it proposes and, where that differs, why. The matrix shows both marks, so a deviation is a distance on the
// grid (P2). One proposal card (DESIGN §16.2): at the desk before the check-in, and at ride-tier type with 72 px
// actions pinned to its bottom edge (always above the fold) once the check-in is done; a remote reaches every cell
// through the wish mode. Safety is never one of the choices.
import { useStore } from "../store";
import { mmss } from "../format";
import { bindings, keyGlyph, type Action } from "../persist";
import { cellName, coherent, FORMAT_NAME, FORMATS, fmtMin, INTENSITIES, INTENSITY_NAME, lockedByCap, sameCell, type Cell, type Wish, type WishOutcome, type WishRecord } from "../mock/wish";
import { WhyChip } from "./Why";

export function Key({ a }: { a: Action }) {
  const k = useStore(bindings)[a][0] ?? "";
  return <kbd>{keyGlyph(k)}</kbd>;
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
      <Matrix o={o} capped={capped} provisional={provisional} onPick={(c) => onWish(sameCell(o.wish, c) ? null : c)} />
      {/* under the matrix, so the matrix stays above the fold at the desk (§16.2); the ↻ cell marks the wish itself */}
      {carried && (
        <p className="carried small" role="status">
          ↻ Carried over from {carried.setOn}: <b>{cellName(carried.cell)}</b> · expires {carried.expires}. <button onClick={onDropCarried}>Drop it</button>
        </p>
      )}
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

/**
 * The one proposal surface (§16.2): outcome line (6 px edge) · session · duration · profile · one reason + Why? ·
 * Start / Easier / Harder / Ride my wish; every other reason and fact sits behind a disclosure. `desk` while the
 * check-in is open; `saddle` (check-in done) is the same card at ride-tier type with 72 px actions pinned to its bottom.
 */
export function ProposalCard(p: {
  mode: "desk" | "saddle";
  o: WishOutcome;
  anyway: boolean;
  askedMin: number;
  name: string;
  totalS: number;
  fallbackReason: string;
  facts: string[];
  ready: boolean;
  /** Why Start is disabled at the desk: not yet confirmed · red flags unanswered · confirmed, check-in still open (§16.2). */
  startHint: "confirm" | "flags" | "close";
  pending: boolean;
  onStart: () => void;
  onAnyway: () => void;
  onStep: (dir: -1 | 1) => void;
  onEdit: () => void;
  children?: React.ReactNode;
}) {
  const saddle = p.mode === "saddle";
  const r = p.anyway ? undefined : p.o.reasons[0];
  const more = p.anyway ? [] : p.o.reasons.slice(1);
  const rel = p.anyway ? "anyway" : p.o.relation;
  const tier = (t: "t3plus" | "t3") => (saddle ? { "data-tier": t, "data-above-fold": "" } : {});
  return (
    <section className={`card proposal-card ${p.mode}`} data-saddle={saddle || undefined} aria-label="Today's proposal" aria-busy={p.pending}>
      <p className={`outcome-line${saddle ? " t3p" : ""} rel-${rel}`} data-above-fold {...tier("t3plus")} data-fit aria-live="polite" data-relation={rel}>
        {outcomeHead(p.o, p.anyway, p.askedMin, !saddle)}
        {p.pending && !saddle && <span className="pending-dot"> · re-planning…</span>}
      </p>
      <h2 className={saddle ? "t3p" : undefined} data-above-fold {...tier("t3plus")} data-fit>
        {p.name} · {mmss(p.totalS)}
      </h2>
      {p.children}
      <p className={`reason-line${saddle ? " t3" : ""}`} {...tier("t3")}>
        {r ? r.text : p.fallbackReason} {r?.rule && <WhyChip id={r.rule} reason={r} />}
        {!p.anyway && p.o.deferredTo && <span className="muted"> Your wish moves to {p.o.deferredTo}{saddle ? "." : ", if your check-in is ok; it expires after 7 days."}</span>}
      </p>
      <details className="more">
        <summary>
          Why today: every fact{more.length ? ` and ${more.length} more reason${more.length === 1 ? "" : "s"}` : ""}
        </summary>
        <ul className="reasons">
          {more.map((x) => (
            <li key={x.text}>
              {x.text} {x.rule && <WhyChip id={x.rule} reason={x} />}
            </li>
          ))}
          {p.facts.map((f) => (
            <li key={f}>{f}</li>
          ))}
        </ul>
        <button className="link" onClick={p.onEdit}>
          Edit a copy in Workouts
        </button>
      </details>
      {saddle && (
        <p className="card-keys t3" data-tier="t3" data-above-fold data-fit>
          <Key a="skip" /> other wish · <Key a="keep" /> coach's pick
        </p>
      )}
      <div className={saddle ? "actions saddle" : "actions"} data-above-fold={saddle ? "" : "wide"} data-fit>
        <button className={saddle ? "primary" : "primary big"} disabled={!p.ready} onClick={p.onStart}>
          <Key a="coach" /> Start
        </button>
        <button onClick={() => p.onStep(-1)}>
          <Key a="minus" /> Easier
        </button>
        <button onClick={() => p.onStep(1)}>
          <Key a="plus" /> Harder
        </button>
        {p.o.overridable && !p.anyway && (
          <button disabled={!p.ready} onClick={p.onAnyway}>
            {p.o.relation === "adapted" ? `Ride the full ${fmtMin(p.askedMin)}` : "Ride my wish"}
          </button>
        )}
      </div>
      {!saddle && !p.ready && (
        <p className="small muted">
          {{ confirm: "Confirm how you feel and answer the health check to start", flags: "Answer the health check to start", close: "Use these answers to finish editing and enable Start" }[p.startHint]}
          {p.o.overridable ? (p.startHint === "close" ? " or ride your wish anyway" : ", or to ride your wish anyway") : ""}.
        </p>
      )}
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
