// The Why? layer and Background (DESIGN §15.3): every claim crank makes about training links to the pack
// rule it rests on, with its evidence grade, the grade of the cut-off crank applied, and the sources.
// The sheet never hides the claim: it pushes the page aside at ≥ 1440 px and repeats the claim below that.
import { useEffect } from "react";
import { compiled, GRADE_WORD, GRADES, refs, rule, rulesOf, topics, type Grade } from "../knowledge";
import { INPUTS, type WishReason } from "../mock/wish";
import { navigate } from "../session";
import { createStore, useStore } from "../store";

/** The open sheet: a rule, and the claim (with the rider's facts) that linked to it, if any. */
export const whyOpen = createStore<{ id: string; reason?: WishReason } | null>(null);
export const openTopic = createStore<string | null>(null);

/** What the grade means for the rider, in words (the grade policy of spec/knowledge-pack/README.md). */
const GRADE_SAYS: Record<Grade, string> = {
  A: "Strong evidence. crank may use it as a default — still fitted to your own numbers.",
  B: "Good evidence. crank may use it as a default.",
  C: "Limited evidence. crank only suggests on it, never acts on it alone, and records how it works out for you.",
  D: "Convention, not evidence: no study supports it yet. crank labels it so, and only uses it where you can ignore it.",
};
/** Inputs the pack's rules mention that crank does not have: the sheet says so instead of implying them. */
const NOT_MEASURED: [RegExp, string][] = [
  [/HRV/, "heart-rate variability (HRV): crank doesn't measure it"],
  [/α1|DFA/, "DFA-α1: crank doesn't use it yet"],
  [/lactate/i, "blood lactate: crank can't measure it"],
];

/** Letter and word, always (P9): the chip never relies on the letter or the outline alone. */
export function GradeChip({ g, note }: { g: Grade; note?: string }) {
  return (
    <span className={`grade g-${g}`} title={GRADES[g].meaning}>
      {g} · {GRADE_WORD[g]}
      {note ? ` ${note}` : ""}
    </span>
  );
}

export function WhyChip({ id, reason }: { id: string; reason?: WishReason }) {
  return (
    <button className="why" onClick={() => whyOpen.set(reason ? { id, reason } : { id })} aria-label={`Why? ${id}`}>
      Why? <GradeChip g={rule(id).grade} />
      {reason?.cutoff && <GradeChip g={reason.cutoff.grade} note="cut-off" />}
    </button>
  );
}

export function WhySheet() {
  const open = useStore(whyOpen);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && whyOpen.set(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);
  if (!open) return null;
  const r = rule(open.id);
  const rs = open.reason;
  const missing = NOT_MEASURED.filter(([re]) => re.test(r.rule) || re.test(r.rationale)).map(([, s]) => s);
  return (
    <aside className="why-sheet" role="dialog" aria-label={`Why: ${r.id}`}>
      <div className="why-body">
        <header>
          <span className="eyebrow">Why? · {r.id}</span>
          <button autoFocus onClick={() => whyOpen.set(null)}>
            Close <kbd>Esc</kbd>
          </button>
        </header>
        {rs && (
          <div className="claim">
            <span className="eyebrow">crank told you</span>
            <p>“{rs.text}”</p>
            {rs.facts.length > 0 && <p className="small">From: {rs.facts.join(" · ")}</p>}
          </div>
        )}
        <h3>The evidence</h3>
        <p className="grade-line">
          <GradeChip g={r.grade} /> {GRADE_SAYS[r.grade]}
        </p>
        <p>{r.rationale}</p>
        {rs?.cutoff && (
          <>
            <h3>The cut-off crank applied</h3>
            <p className="grade-line">
              <GradeChip g={rs.cutoff.grade} /> {rs.cutoff.what}: a convention, not a finding. The direction rests on the evidence above; where to draw the line doesn't — which is why you can ride your wish anyway.
            </p>
          </>
        )}
        <h3>What the pack recommends</h3>
        <p className="muted">{r.rule}</p>
        <h3>What crank uses here</h3>
        <p>
          {rs?.uses.length ? rs.uses.map((u) => INPUTS[u]).join(", ") : "Nothing of yours yet: this is background."}
          {missing.length > 0 && <span className="muted"> Not used: {missing.join("; ")}.</span>}
        </p>
        <p className="safety-first small">
          <b>Safety comes first.</b> Chest pain, fainting, a racing heartbeat with dizziness or unusual breathlessness are not a readiness question: they stop every plan until a doctor clears you (Settings → Health).
        </p>
        <h3>Sources</h3>
        {r.refs.length === 0 ? (
          <p className="muted">None: this is a convention, not a finding.</p>
        ) : (
          <ol className="sources">
            {r.refs.map((k) => {
              const s = refs.get(k);
              return (
                <li key={k}>
                  {s?.authors} ({s?.year}). {s?.title}. <i>{s?.journal}</i>. {s?.doi ? <a href={`https://doi.org/${s.doi}`}>doi.org/{s.doi}</a> : s?.pmid ? `PMID ${s.pmid}` : null}
                </li>
              );
            })}
          </ol>
        )}
        {r.topics.map((no) => (
          <button key={no} className="link" onClick={() => (whyOpen.set(null), openTopic.set(no), navigate("settings", "background"))}>
            Read the background: {topics.find((t) => t.no === no)?.title} →
          </button>
        ))}
        <p className="small muted foot">From crank's knowledge pack ({compiled}) · works offline; source links need a connection · training guidance for healthy riders, not medical advice.</p>
      </div>
    </aside>
  );
}

/** A finding's grade as written in the pack ("B (observational)", "A-"): letter · word, then the qualifier. */
function FindingGrade({ g }: { g: string }) {
  const letter = (g[0] ?? "D") as Grade;
  return <GradeChip g={letter} note={g.slice(1).trim()} />;
}

/** Settings → Background: the whole pack, browsable by topic (DESIGN §15.3). */
export function Background() {
  const no = useStore(openTopic);
  const t = topics.find((x) => x.no === no);
  if (t)
    return (
      <>
        <button className="link" onClick={() => openTopic.set(null)}>
          ← All topics
        </button>
        <h2>
          {t.no} · {t.title}
        </h2>
        <p className="muted">How sure: {t.confidence.replaceAll("**", "")}</p>
        <h3>What the research says</h3>
        <ul className="findings">
          {t.findings.map((f) => (
            <li key={f.headline}>
              <FindingGrade g={f.grade} /> {f.headline} <span className="muted small">· {f.cites} {f.cites === 1 ? "source" : "sources"}</span>
            </li>
          ))}
        </ul>
        <h3>What the pack recommends</h3>
        <ul className="findings">
          {rulesOf(t.no).map((r) => (
            <li key={r.id}>
              <span className="muted small">{r.id}</span> {r.rule} <WhyChip id={r.id} />
            </li>
          ))}
        </ul>
      </>
    );
  return (
    <>
      <h2>Background</h2>
      <p className="muted">The research behind crank's coach. Every rule carries how strong its evidence is, and its sources. Compiled {compiled} from {refs.size} papers.</p>
      <dl className="grades">
        {(["A", "B", "C", "D"] as const).map((g) => (
          <div key={g}>
            <dt>
              <GradeChip g={g} />
            </dt>
            <dd>{GRADE_SAYS[g]}</dd>
          </div>
        ))}
        <div>
          <dt className="small muted">“A-”, “(observational)”</dt>
          <dd>qualifiers from the pack: a review without a pooled result, or evidence from observing athletes rather than trials.</dd>
        </div>
      </dl>
      <h3>Topics</h3>
      <ul className="topics">
        {topics.map((x) => (
          <li key={x.no}>
            <button onClick={() => openTopic.set(x.no)}>
              <b>
                {x.no} · {x.title}
              </b>
              <span className="grades-row">
                {(["A", "B", "C", "D"] as const).map((g) => {
                  const k = rulesOf(x.no).filter((r) => r.grade === g).length;
                  return k ? <GradeChip key={g} g={g} note={`× ${k}`} /> : null;
                })}
              </span>
            </button>
          </li>
        ))}
      </ul>
      <p className="small muted">Works offline; source links need a connection. crank is not a medical device: training guidance for healthy riders, not medical advice.</p>
    </>
  );
}
