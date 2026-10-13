// The Why? layer and Background (DESIGN §15.3): every claim crank makes about training links to the pack
// rule it rests on, with its evidence grade and sources. Desk reading, never shown mid-ride.
import { useEffect } from "react";
import { compiled, GRADE_WORD, GRADES, refs, rule, rulesOf, topics, type Grade } from "../knowledge";
import { navigate } from "../session";
import { createStore, useStore } from "../store";

export const whyOpen = createStore<string | null>(null);
export const openTopic = createStore<string | null>(null);

/** What the grade means for the rider, in words (the grade policy of spec/knowledge-pack/README.md). */
const GRADE_SAYS: Record<Grade, string> = {
  A: "Strong evidence. crank may use it as a default — still fitted to your own numbers.",
  B: "Good evidence. crank may use it as a default.",
  C: "Limited evidence. crank only suggests on it, never acts on it alone, and records how it works out for you.",
  D: "Convention, not evidence: no study supports it yet. crank labels it so, and only uses it where you can ignore it.",
};

export function GradeChip({ g }: { g: Grade }) {
  return (
    <span className={`grade g-${g}`} title={GRADES[g].meaning}>
      {g} · {GRADE_WORD[g]}
    </span>
  );
}

export function WhyChip({ id }: { id: string }) {
  return (
    <button className="why" onClick={() => whyOpen.set(id)} aria-label={`Why? ${id}`}>
      Why? <GradeChip g={rule(id).grade} />
    </button>
  );
}

export function WhySheet() {
  const id = useStore(whyOpen);
  useEffect(() => {
    if (!id) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && whyOpen.set(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [id]);
  if (!id) return null;
  const r = rule(id);
  return (
    <aside className="why-sheet" role="dialog" aria-label={`Why: ${r.id}`}>
      <div className="why-body">
      <header>
        <span className="eyebrow">Why? · {r.id}</span>
        <button autoFocus onClick={() => whyOpen.set(null)}>
          Close <kbd>Esc</kbd>
        </button>
      </header>
      <p className="grade-line">
        <GradeChip g={r.grade} /> {GRADE_SAYS[r.grade]}
      </p>
      <h3>Why</h3>
      <p>{r.rationale}</p>
      <h3>What crank does</h3>
      <p className="muted">{r.rule}</p>
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
              <span className={`grade g-${f.grade[0]}`}>{f.grade}</span> {f.headline} <span className="muted small">· {f.cites} {f.cites === 1 ? "source" : "sources"}</span>
            </li>
          ))}
        </ul>
        <h3>What crank does with it</h3>
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
      <p className="muted">The research behind crank's coach. Every rule it uses carries how strong its evidence is, and its sources. Compiled {compiled} from {refs.size} papers.</p>
      <dl className="grades">
        {(["A", "B", "C", "D"] as const).map((g) => (
          <div key={g}>
            <dt>
              <GradeChip g={g} />
            </dt>
            <dd>{GRADE_SAYS[g]}</dd>
          </div>
        ))}
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
                  const n = rulesOf(x.no).filter((r) => r.grade === g).length;
                  return n ? <span key={g} className={`grade g-${g}`}>{`${n} × ${g}`}</span> : null;
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
