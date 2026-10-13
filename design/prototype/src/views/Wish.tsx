// The planner wish (DESIGN §15.2): the rider points into intensity × format; the planner answers with
// what it proposes and, where that differs, why. The matrix shows both marks, so a deviation is a
// distance on the grid, not a sentence to decode (P2). Safety is never one of the choices.
import { useStore } from "../store";
import { bindings, type Action } from "../persist";
import { cellName, coherent, FORMAT_NAME, FORMATS, INTENSITIES, INTENSITY_NAME, lockedByCap, type Cell, type Wish, type WishOutcome } from "../mock/wish";
import { WhyChip } from "./Why";

const same = (a: Cell | null, b: Cell | null) => !!a && !!b && a.intensity === b.intensity && a.format === b.format;

function Key({ a }: { a: Action }) {
  const k = useStore(bindings)[a][0] ?? "";
  return <kbd>{k === "Backspace" ? "⌫" : k}</kbd>;
}

export function WishCard({ wish, outcome, capped, onWish }: { wish: Wish; outcome: WishOutcome; capped: boolean; onWish: (w: Wish) => void }) {
  return (
    <section className="card wish" aria-label="Your wish">
      <h2>Anything you'd like to ride?</h2>
      <p className="wish-now" data-fit>
        {wish ? <>Wish: <b>{cellName(wish)}</b></> : <>No wish — <b>the coach picks</b></>}
      </p>
      <div className="matrix" role="grid" aria-label="Wish: intensity × format">
        <span />
        {FORMATS.map((f) => (
          <span key={f} className="axis" role="columnheader">
            {FORMAT_NAME[f]}
          </span>
        ))}
        {[...INTENSITIES].reverse().map((i) => (
          <div key={i} role="row" className="matrix-row">
            <span className="axis" role="rowheader">
              {INTENSITY_NAME[i][0]}
              <small>{INTENSITY_NAME[i][1]}</small>
            </span>
            {FORMATS.map((f) => {
              const c: Cell = { intensity: i, format: f };
              if (!coherent(c)) return <span key={f} className="cell none" aria-hidden />;
              const locked = lockedByCap(c, capped);
              const isWish = same(wish, c);
              const isPlan = same(outcome.delivered, c);
              return (
                <button
                  key={f}
                  role="gridcell"
                  className={`cell${isWish ? " wished" : ""}${isPlan ? " planned" : ""}${locked ? " locked" : ""}`}
                  aria-selected={isWish}
                  aria-label={`${cellName(c)}${locked ? ", above your cap" : ""}`}
                  disabled={locked}
                  data-cell={`${i}.${f}`}
                  onClick={() => onWish(isWish ? null : c)}
                >
                  {locked ? "cap" : isWish && isPlan ? "✓ wish" : isWish ? "wish" : isPlan ? (wish ? "plan" : "coach") : ""}
                </button>
              );
            })}
          </div>
        ))}
      </div>
      <p className="small muted">
        {capped ? "VO2max and sprints are off while your injury cap is on. " : ""}From the saddle: <Key a="minus" /> <Key a="plus" /> easier · harder, <Key a="keep" /> back to the coach's pick.
      </p>
      {wish && <button onClick={() => onWish(null)}>Let the coach pick</button>}
    </section>
  );
}

/** How the planner treated the wish, in the proposal card: facts, then the rule and its grade. */
export function WishOutcomeBlock({ o, anyway, onAnyway }: { o: WishOutcome; anyway: boolean; onAnyway: () => void }) {
  if (!o.wish || o.relation === "no_wish") return <p className="wish-line muted">No wish: this is the coach's pick for today.</p>;
  const steps = INTENSITIES.indexOf(o.wish.intensity) - INTENSITIES.indexOf(o.delivered.intensity);
  const head = anyway
    ? `Your wish, as you chose: ${cellName(o.wish)} — logged. The coach's in-ride limits still apply.`
    : o.relation === "as_wished"
      ? `✓ As you wished: ${cellName(o.wish)}`
      : o.relation === "adapted"
        ? `≈ Your wish, adapted: ${cellName(o.delivered)}`
        : `You wished ${cellName(o.wish)} → proposed ${cellName(o.delivered)}, ${steps} step${steps === 1 ? "" : "s"} easier`;
  return (
    <div className={`wish-outcome rel-${anyway ? "anyway" : o.relation}`} data-relation={anyway ? "anyway" : o.relation}>
      <p className="wish-line">
        <b>{head}</b>
      </p>
      {!anyway && o.reasons.length > 0 && (
        <ul className="reasons">
          {o.reasons.map((r) => (
            <li key={r.text}>
              {r.text} {r.rule && <WhyChip id={r.rule} />}
            </li>
          ))}
        </ul>
      )}
      {!anyway && o.deferredTo && <p className="small">Your wish moves to {o.deferredTo}.</p>}
      {!anyway && o.overridable && (
        <div className="actions">
          <button onClick={onAnyway}>Ride my wish anyway</button>
          <span className="small muted">your choice, logged with these reasons</span>
        </div>
      )}
    </div>
  );
}
