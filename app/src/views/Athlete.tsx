// Settings → Athlete model (ADR-0009 §2.1.3, DESIGN §5.9 "Athlete model", #716): the suggested values at the top, then
// one row per parameter in ADR-0009 §2.1.1's order: its value ± sd (or what it is without one), where it came from and
// when, a stale mark after 6 weeks, and Edit, History and Clear. Edit opens the row's draft (settings/draft.ts): the
// value and an optional ±, Save and Cancel. History lists every value with its source and date and "Use this value".
// Every command answers nothing; the page reads the model again (athleteHr.ts `refreshAthleteModel`) and, with the
// history open, the history. A refusal is a notice in the row; a failed read is a notice under the section head.
import { useEffect, useState } from "react";
import { api, type Result } from "../api";
import {
  refreshAthleteModel,
  useAthleteHr,
  useAthleteModel,
} from "../athleteHr";
import {
  commands,
  type ApiError,
  type AthleteParam,
  type AthleteParamState,
  type AthleteProposalView,
  type EstimateView,
  type HistoryEntry,
  type ResolvedView,
} from "../bindings";
import { Num } from "../Num";
import { useDraft } from "../settings/draft";
import { useCommand } from "../stream";

/** The ride's HR-zone hint (§18.3 "Heart-rate zone"): one line, shown while neither reference is known. */
export const NO_HR_REFS =
  "No max or threshold heart rate yet, so the ride shows no heart-rate zone.";

/** How a parameter reads: its name, the unit it is shown and typed in, that unit per stored unit, and its decimals. */
const META: Record<
  AthleteParam,
  { label: string; unit: string; per: number; digits: number }
> = {
  ftp: { label: "FTP", unit: "W", per: 1, digits: 0 },
  cp: { label: "Critical power (CP)", unit: "W", per: 1, digits: 0 },
  w_prime: { label: "W′ (work above CP)", unit: "kJ", per: 1000, digits: 1 },
  pmax: { label: "Peak power", unit: "W", per: 1, digits: 0 },
  hr_max: { label: "Max heart rate", unit: "bpm", per: 1, digits: 0 },
  lthr: {
    label: "Threshold heart rate (LTHR)",
    unit: "bpm",
    per: 1,
    digits: 0,
  },
  hr_rest: { label: "Resting heart rate", unit: "bpm", per: 1, digits: 0 },
  weight: { label: "Weight", unit: "kg", per: 1, digits: 1 },
};

/** An estimate older than this is drawn stale (ADR-0009 §2.1.2, DESIGN §5.6). */
const STALE_DAYS = 42;
const DAY_S = 86_400;

const num = (param: AthleteParam, stored: number) =>
  (stored / META[param].per).toFixed(META[param].digits);
/** "287 W", "20.0 kJ". */
const amount = (param: AthleteParam, stored: number) =>
  `${num(param, stored)} ${META[param].unit}`;
/** "287 W ± 12". */
const withSd = (param: AthleteParam, value: number, sd: number | null) =>
  sd === null
    ? amount(param, value)
    : `${amount(param, value)} ± ${num(param, sd)}`;

/** The day of a row, "Mar 3" (the year too when it is not this one). */
function day(at: number, now: number): string {
  const d = new Date(at * 1000);
  const thisYear = d.getFullYear() === new Date(now).getFullYear();
  return d.toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    ...(thisYear ? {} : { year: "numeric" }),
  });
}

/** "today", "3 days ago", "8 weeks ago" (Intl.RelativeTimeFormat). */
function age(at: number, now: number): string {
  const days = Math.max(0, Math.floor((now / 1000 - at) / DAY_S));
  const rel = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  return days < 14
    ? rel.format(-days, "day")
    : rel.format(-Math.floor(days / 7), "week");
}

/** Where a value came from, in words: "typed in", "ramp test", "from your rides". */
function origin(e: Pick<EstimateView, "source" | "method">): string {
  switch (e.source) {
    case "self_report":
      return "typed in";
    case "test":
      return e.method.startsWith("ramp") ? "ramp test" : "test";
    case "rides":
      return "from your rides";
    case "imported":
      return "imported";
    case "prior":
      return "estimate";
  }
}

/** What the row says the parameter is: "287 W ± 12", "estimate: 250 W (no test yet)", "Not set". */
function valueText(param: AthleteParam, r: ResolvedView | null): string {
  if (r === null || r.value === null) return "Not set";
  if (r.source === "prior")
    return `estimate: ${amount(param, r.value)} (${param === "hr_max" ? "from your age" : "no test yet"})`;
  return withSd(param, r.value, r.sd);
}

/** A refusal, as a notice: ⚠ and the core's message (§18.7 rule 11), and Retry where it helps. */
function Notice({ error, onRetry }: { error: ApiError; onRetry?: () => void }) {
  return (
    <div role="alert" className="device-notice" data-state="error">
      <p>
        <span data-state-label>⚠</span> {error.message}
      </p>
      {onRetry && (
        <button type="button" onClick={onRetry}>
          Try again
        </button>
      )}
    </div>
  );
}

interface Typed {
  value: string;
  sd: string;
}
/** A typed number: "287", "72,5"; `null` when it is none. */
const parse = (s: string): number | null => {
  const t = s.trim().replace(",", ".");
  if (t === "" || !/^\d*\.?\d+$|^\d+\.$/.test(t)) return null;
  return Number(t);
};

/** The row's draft (ADR-0009 §2.1.3 "Edit"): the value and an optional ±, Save and Cancel. */
function EditForm({
  param,
  current,
  onDone,
}: {
  param: AthleteParam;
  current: ResolvedView | null;
  onDone: (saved: boolean) => void;
}) {
  const m = META[param];
  const [draft, view] = useDraft<Typed>(
    () => ({
      value:
        current && current.value !== null && current.source !== "prior"
          ? num(param, current.value)
          : "",
      sd: "",
    }),
    (t) =>
      parse(t.value) === null
        ? `Type the ${m.label === "FTP" ? "FTP" : "value"} in ${m.unit}.`
        : t.sd.trim() !== "" && parse(t.sd) === null
          ? `Type the ± in ${m.unit}, or leave it empty.`
          : null,
  );
  const save = async () => {
    const saved = await draft.save(async (t) => {
      const sd = t.sd.trim() === "" ? null : parse(t.sd)! * m.per;
      const r = await api(
        commands.athleteSet(param, parse(t.value)! * m.per, sd),
      );
      return r.status === "ok" ? null : r.error.message;
    });
    if (saved) onDone(true);
  };
  return (
    <form
      className="athlete-edit"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <div className="athlete-fields">
        <label className="athlete-field">
          <input
            type="text"
            inputMode="decimal"
            aria-label={`${m.label}, ${m.unit}`}
            value={view.draft.value}
            onChange={(e) =>
              draft.set({ ...view.draft, value: e.target.value })
            }
            // The field the rider opened Edit for (§18.4: the form opens in place of the row's value).
            // eslint-disable-next-line jsx-a11y/no-autofocus -- focus moves to the control Edit opened
            autoFocus
          />
          <span>{m.unit}</span>
        </label>
        <label className="athlete-field">
          <span>±</span>
          <input
            type="text"
            inputMode="decimal"
            aria-label={`± ${m.unit}, optional`}
            placeholder="20 %"
            value={view.draft.sd}
            onChange={(e) => draft.set({ ...view.draft, sd: e.target.value })}
          />
        </label>
        <button
          type="submit"
          className="primary"
          disabled={view.saving}
          aria-busy={view.saving || undefined}
        >
          {view.saving ? "Saving…" : "Save"}
        </button>
        <button
          type="button"
          disabled={view.saving}
          onClick={() => {
            draft.cancel();
            onDone(false);
          }}
        >
          Cancel
        </button>
      </div>
      {view.error && (
        <p role="alert" className="athlete-error">
          <span data-state-label>⚠</span> Not saved: {view.error}
        </p>
      )}
    </form>
  );
}

/** A parameter's history, newest first, with "Use this value" on each earlier value that is not a prior. */
function History({
  param,
  entries,
  busy,
  onUse,
  now,
}: {
  param: AthleteParam;
  entries: HistoryEntry[];
  busy: boolean;
  onUse: (id: number) => void;
  now: number;
}) {
  if (entries.length === 0)
    return (
      <p className="athlete-history-empty">
        Nothing yet: a value you type in, or a test result, starts the history.
      </p>
    );
  return (
    <ol className="athlete-history" aria-label={`${META[param].label} history`}>
      {entries.map((e, i) => (
        <li key={e.id}>
          <Num className="athlete-history-value">
            {e.kind === "cleared"
              ? "Cleared"
              : e.value === null
                ? "—"
                : withSd(param, e.value, e.source === "prior" ? null : e.sd)}
          </Num>
          <span className="athlete-history-source">
            {e.kind === "cleared"
              ? "not set from then"
              : e.source === "prior"
                ? "cleared to the estimate"
                : origin(e)}
            {e.updatedAt !== null && `, ${day(e.updatedAt, now)}`}
          </span>
          {i === 0 ? (
            <span className="athlete-history-current">Current</span>
          ) : e.kind === "estimate" && e.source !== "prior" ? (
            <button type="button" disabled={busy} onClick={() => onUse(e.id)}>
              Use this value
            </button>
          ) : (
            <span />
          )}
        </li>
      ))}
    </ol>
  );
}

/**
 * One parameter: its value, source and age, and its actions; or its draft, in place. Each action's name says the
 * parameter after its visible word ("Edit FTP"), so eight rows' buttons, and the rail's History, are told apart.
 */
function Row({ state, now }: { state: AthleteParamState; now: number }) {
  const { param, resolved, estimate } = state;
  const m = META[param];
  const [editing, setEditing] = useState(false);
  const [showHistory, setShowHistory] = useState(false);
  const [refusal, setRefusal] = useState<ApiError | null>(null);
  const [busy, setBusy] = useState(false);
  const history = useCommand(commands.athleteHistory);
  const { run: runHistory } = history;
  useEffect(() => {
    if (showHistory) void runHistory(param);
  }, [showHistory, runHistory, param]);
  const changed = async () => {
    await refreshAthleteModel();
    if (showHistory) await runHistory(param);
  };
  const act = async (call: Promise<Result<null>>) => {
    setBusy(true);
    const r = await api(call);
    setBusy(false);
    setRefusal(r.status === "ok" ? null : r.error);
    if (r.status === "ok") await changed();
  };
  const measured = resolved !== null && resolved.source !== "prior";
  const at = estimate?.updatedAt ?? null;
  const stale = measured && at !== null && now / 1000 - at > STALE_DAYS * DAY_S;
  const headId = `athlete-${param}-l`;
  return (
    <li className="athlete-row" aria-labelledby={headId}>
      <span id={headId} className="setting-label">
        {m.label}
      </span>
      {editing ? (
        <EditForm
          param={param}
          current={resolved}
          onDone={(saved) => {
            setEditing(false);
            if (saved) {
              setRefusal(null);
              void changed();
            }
          }}
        />
      ) : (
        <>
          <div className="athlete-value">
            <Num data-testid={`athlete-${param}`}>
              {valueText(param, resolved)}
            </Num>
            <span
              className="athlete-source"
              data-testid={`athlete-${param}-source`}
            >
              {measured && estimate
                ? `${origin(estimate)}${at === null ? "" : `, ${day(at, now)} · ${age(at, now)}`}`
                : "not measured yet"}
              {stale && (
                <span
                  className="athlete-stale"
                  title="Over 6 weeks old: a new test would check it"
                >
                  {" "}
                  · stale
                </span>
              )}
            </span>
          </div>
          <div className="device-actions">
            <button
              type="button"
              disabled={busy}
              aria-label={`Edit ${m.label}`}
              onClick={() => setEditing(true)}
            >
              Edit
            </button>
            <button
              type="button"
              aria-expanded={showHistory}
              aria-label={`History of ${m.label}`}
              onClick={() => setShowHistory((s) => !s)}
            >
              History
            </button>
            <button
              type="button"
              className="danger"
              disabled={busy || !measured}
              aria-label={`Clear ${m.label}`}
              onClick={() => void act(commands.athleteClear(param))}
            >
              Clear
            </button>
          </div>
        </>
      )}
      {refusal && (
        <div className="athlete-wide">
          <Notice error={refusal} />
        </div>
      )}
      {showHistory && (
        <div className="athlete-wide">
          {history.error ? (
            <Notice
              error={history.error}
              onRetry={() => void runHistory(param)}
            />
          ) : history.data ? (
            <History
              param={param}
              entries={history.data}
              busy={busy}
              onUse={(id) => void act(commands.athleteRevert(id))}
              now={now}
            />
          ) : (
            <p className="athlete-history-empty">Loading…</p>
          )}
        </div>
      )}
    </li>
  );
}

/** The open proposals (ADR-0009 §2.2.4). Accept and Dismiss are #720's commands; their buttons land with #753. */
function Proposals({
  proposals,
  now,
}: {
  proposals: AthleteProposalView[];
  now: number;
}) {
  return (
    <div className="device-group">
      <div className="device-group-head">
        <h3>Suggested values</h3>
      </div>
      {proposals.length === 0 ? (
        <p className="device-hint">
          None. When a test or your rides show a new value, it is suggested
          here, and nothing changes until you accept it.
        </p>
      ) : (
        <ul className="athlete-proposals">
          {proposals.map((p) => (
            <li key={p.id}>
              <span>
                {p.values.map((v, i) => (
                  <span key={v.param}>
                    {i > 0 && ", "}
                    {META[v.param].label}
                    {v.value !== null && (
                      <>
                        {" "}
                        <Num>{withSd(v.param, v.value, v.sd)}</Num>
                      </>
                    )}
                  </span>
                ))}
              </span>
              {p.createdAt !== null && (
                <span className="athlete-source">{`suggested ${day(p.createdAt, now)}`}</span>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export function Athlete() {
  const { model, error } = useAthleteModel();
  const { hrMaxBpm, lthrBpm } = useAthleteHr();
  // The clock of the dates and ages, read once (as Today's): a row's age is of the moment the section opened.
  const [now] = useState(() => Date.now());
  return (
    <section className="settings-section" aria-labelledby="athlete-h">
      <div className="section-head">
        <h2 id="athlete-h">Athlete model</h2>
        <p className="settings-note">
          The numbers your workouts and zones are set from. A ride uses the
          values of the moment it is loaded.
        </p>
      </div>
      {error && (
        <Notice error={error} onRetry={() => void refreshAthleteModel()} />
      )}
      {model && (
        <>
          <Proposals proposals={model.proposals} now={now} />
          <ul className="athlete-rows" aria-label="Parameters">
            {model.params.map((p) => (
              <Row key={p.param} state={p} now={now} />
            ))}
          </ul>
          {hrMaxBpm === null && lthrBpm === null && (
            <p className="settings-note">{NO_HR_REFS}</p>
          )}
        </>
      )}
      {!model && !error && <p className="settings-note">Loading…</p>}
    </section>
  );
}
