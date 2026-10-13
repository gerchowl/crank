import type { Filter } from "@neo/shared";

const KINDS: Filter["kind"][] = ["ema", "sma", "median", "one_euro", "hold"];
function make(kind: Filter["kind"]): Filter {
  switch (kind) {
    case "ema":
      return { kind, tauS: 2 };
    case "sma":
      return { kind, windowS: 3 };
    case "median":
      return { kind, n: 5 };
    case "one_euro":
      return { kind, minCutoff: 0.5, beta: 0.01 };
    case "hold":
      return { kind };
  }
}

function Num({ label, value, step, onChange }: { label: string; value: number; step: number; onChange: (v: number) => void }) {
  return (
    <label className="inline">
      {label}
      <input type="number" value={value} step={step} min={0} onChange={(e) => { onChange(Number(e.target.value)); }} />
    </label>
  );
}

/** Filter chain (applied in order on every source sample, backend-side). */
export function FilterChainEditor({ value, onChange }: { value: Filter[]; onChange: (f: Filter[]) => void }) {
  const set = (i: number, f: Filter) => {
    onChange(value.map((x, k) => (k === i ? f : x)));
  };
  return (
    <fieldset className="filters">
      <legend>Filter chain</legend>
      {value.length === 0 && <p className="muted small">raw</p>}
      {value.map((f, i) => (
        <div key={i} className="filter-row">
          <select value={f.kind} onChange={(e) => { set(i, make(e.target.value as Filter["kind"])); }}>
            {KINDS.map((k) => <option key={k} value={k}>{k}</option>)}
          </select>
          {f.kind === "ema" && <Num label="τ s" value={f.tauS} step={0.5} onChange={(v) => { set(i, { ...f, tauS: v }); }} />}
          {f.kind === "sma" && <Num label="win s" value={f.windowS} step={1} onChange={(v) => { set(i, { ...f, windowS: v }); }} />}
          {f.kind === "median" && <Num label="n" value={f.n} step={1} onChange={(v) => { set(i, { ...f, n: v }); }} />}
          {f.kind === "one_euro" && (
            <>
              <Num label="fc" value={f.minCutoff} step={0.1} onChange={(v) => { set(i, { ...f, minCutoff: v }); }} />
              <Num label="β" value={f.beta} step={0.005} onChange={(v) => { set(i, { ...f, beta: v }); }} />
            </>
          )}
          <button type="button" className="icon-btn" title="Remove filter" onClick={() => { onChange(value.filter((_, k) => k !== i)); }}>✕</button>
        </div>
      ))}
      <button type="button" className="btn small" onClick={() => { onChange([...value, make("ema")]); }}>+ filter</button>
    </fieldset>
  );
}
