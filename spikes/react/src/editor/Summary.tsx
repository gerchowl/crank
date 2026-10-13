import { backend } from "../backend";
import { mmss } from "../lib/format";
import { useEditor } from "./context";

/** Summary from compileWorkout: duration (range), TSS, IF, time-in-zone bars, validation issues. */
export function Summary() {
  const { compiled, dispatch } = useEditor();
  const c = compiled;
  const max = Math.max(1, ...c.timeInZoneS);
  const [lo, hi] = c.totalRangeS;
  return (
    <section className="summary">
      <div className="tiles">
        <div><span className="label">Duration</span><b>{mmss(c.totalS)}</b>{lo !== hi && <span className="muted small">{mmss(lo)}–{mmss(hi)}</span>}</div>
        <div><span className="label">TSS</span><b>{Math.round(c.tss)}</b></div>
        <div><span className="label">IF</span><b>{c.if.toFixed(2)}</b></div>
      </div>
      <div className="tiz" role="table" aria-label="time in zone">
        {c.timeInZoneS.map((s, i) => (
          <div key={i} className="tiz-row" role="row">
            <span role="cell" className="tiz-z">{backend.zones.power[i]?.name.split(" ")[0] ?? `Z${String(i + 1)}`}</span>
            <span role="cell" className="tiz-bar"><i style={{ width: `${String((s / max) * 100)}%`, background: `var(--zone-${String(i + 1)})` }} /></span>
            <span role="cell" className="tiz-v">{mmss(s)}</span>
          </div>
        ))}
      </div>
      <ul className="issues">
        {c.issues.length === 0 && <li className="muted"><span className="status-tag good">✓ valid</span> no issues</li>}
        {c.issues.map((iss, i) => (
          <li key={i}>
            <button type="button" className="linkish" onClick={() => { if (iss.nodeId) dispatch({ type: "select", ids: [iss.nodeId] }); }}>
              <span className={`status-tag ${iss.severity === "error" ? "critical" : "warning"}`}>{iss.severity === "error" ? "✕ error" : "! warning"}</span> {iss.message}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
