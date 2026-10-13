// Review (DESIGN §5.5) and History (§5.6).
import { useState } from "react";
import type { CoachNudge, MetricId, RideStreams } from "@crank/shared";
import { Bars, gaps, Panel, PdCurve, Profile, TimeAxis, Trend } from "../charts";
import { mmss, num, signed } from "../format";
import { athleteModel, calibration, coachReason, cpTrend, intentVerdict, metricValid, pastRides, sceneFixture, tauSlower, workIntervals } from "../mock/ext";
import { backend, navigate, noStatus, usePoll, useMetric, useSession, type RideSession } from "../session";
import { useStore } from "../store";

export function Review() {
  const s = useSession();
  if (!s) return <p className="empty">No ride to review yet.</p>;
  return <RideReview s={s} />;
}

const TILES: [MetricId, string, string, number][] = [
  ["elapsed", "Time", "", 0],
  ["kj", "Work", "kJ", 0],
  ["np", "NP", "W", 0],
  ["tss", "TSS", "", 0],
  ["if", "IF", "", 2],
  ["compliance", "On target", "%", 0],
  ["decoupling", "Decoupling", "%", 1],
];
const RPE_WORDS: Record<number, string> = { 1: "very easy", 4: "moderate", 7: "hard", 10: "max" };

function Tile({ metric, label, unit, digits }: { metric: MetricId; label: string; unit: string; digits: number }) {
  const v = useMetric(metric, 0.5);
  return (
    <div className="tile">
      <span className="lbl">{label}</span>
      <b>
        {metric === "elapsed" ? mmss(v) : num(v, digits)}
        <small>{unit}</small>
      </b>
    </div>
  );
}

/** The 3 minutes around a coach call (1 min before, 2 after), so the rating has context. */
function CallContext({ streams, n, no }: { streams: RideStreams; n: CoachNudge; no: number }) {
  const from = Math.max(0, Math.round(n.t) - 60);
  const to = Math.min(streams.toS, Math.round(n.t) + 120);
  const cut = (m: MetricId) => (streams.series[m] ?? []).slice(from, to);
  return (
    <div className="call-context">
      <p className="muted">{n.message}</p>
      <Panel n={to - from} fromS={from} unit="W" height={60} markers={[Math.round(n.t)]} markerNo={[no]} series={[{ values: cut("target_power"), cls: "target", label: "target", area: true }, { values: cut("power"), cls: "series-power", label: "power" }]} />
      <Panel n={to - from} fromS={from} unit="bpm" height={44} markers={[Math.round(n.t)]} markerNo={[no]} series={[{ values: cut("hr"), cls: "series-hr", label: "heart rate" }]} />
      <TimeAxis n={to - from} fromS={from} />
    </div>
  );
}

function RideTable({ streams }: { streams: RideStreams }) {
  const rows = [];
  for (let i = 0; i < streams.toS; i += 60) rows.push(i);
  const v = (m: MetricId, i: number) => num(streams.series[m]?.[i] ?? null);
  return (
    <table className="ride-table">
      <thead>
        <tr>
          <th>time</th>
          <th>target W</th>
          <th>power W</th>
          <th>HR bpm</th>
          <th>cadence rpm</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((i) => (
          <tr key={i}>
            <td>{mmss(i)}</td>
            <td>{v("target_power", i)}</td>
            <td>{v("power", i)}</td>
            <td>{v("hr", i)}</td>
            <td>{v("cadence", i)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function RideReview({ s }: { s: RideSession }) {
  const nudges = useStore(s.nudges);
  const reverted = useStore(s.reverted);
  const current = useStore(s.current);
  const status = useStore(s.status);
  const [rpe, setRpe] = useState<number | null>(null);
  const [rated, setRated] = useState<Record<string, string>>({});
  const [open, setOpen] = useState<string | null>(() => {
    const i = sceneFixture.get().openCall;
    return i === undefined ? null : (s.nudges.get()[i]?.callId ?? null);
  });
  const [table, setTable] = useState(false);
  const [allNumbers, setAllNumbers] = useState(false);
  const streams = usePoll(1, () => backend.streams(["power", "target_power", "hr", "cadence"]), []);
  const recoveries = usePoll(0.2, () => backend.recoveries(), []);
  const mmp = usePoll(0.2, () => backend.mmp(), []);
  const hist = usePoll(0.2, () => backend.histogram("power", 20), []);
  if (!current) return null;
  const applied = nudges.filter((n) => (n.status === "auto_applied" || n.status === "accepted") && !reverted.has(n.callId));
  const series = streams.series;
  const n = streams.toS - streams.fromS;
  const markers = nudges.map((x) => Math.round(x.t));
  const laps = workIntervals(current.workout, current.compiled);
  const finished = status?.state === "finished";
  const verdict = intentVerdict(current.compiled, recoveries);
  const noPower = gaps(series.power ?? []).reduce((a, [x, y]) => a + (y - x), 0);
  const rate = (callId: string, r: "right" | "wrong" | "unsure") => {
    backend.coach.rate(callId, r, true);
    setRated((p) => ({ ...p, [callId]: r }));
  };

  return (
    <div className="page review">
      <header className="page-head">
        <span className="eyebrow">{finished ? "Ride complete" : "Ride ended early"} · {mmss(status?.t)}</span>
        <h1>{current.workout.name}</h1>
      </header>
      <section className="card verdict" aria-label="Did it work">
        <h2>Did it do its job?</h2>
        <ul className="verdict-list">
          {verdict.map((v) => (
            <li key={v.part} className={v.ok ? "ok" : "warn"}>
              <span aria-hidden>{v.ok ? "✓" : "!"}</span> <b>{v.part}</b> — {v.verdict}
            </li>
          ))}
          <li className="ok">
            <span aria-hidden>i</span> <b>Coach</b> — {applied.length ? `${applied.length} change applied (${applied.map((a) => `${signed(a.proposedPctChange ?? 0)} %`).join(", ")})` : "no changes applied"}, {nudges.length - applied.length} not taken
          </li>
          {noPower > 0 && (
            <li className="warn">
              <span aria-hidden>!</span> <b>Data</b> — {mmss(noPower)} without power data (trainer reconnect), excluded from "on target"
            </li>
          )}
        </ul>
      </section>

      <section className="card inputs" aria-label="Two quick inputs">
        <div>
          <h2>How hard was it?</h2>
          <div className="rpe" role="radiogroup" aria-label="Rating of perceived exertion">
            {Array.from({ length: 10 }, (_, i) => i + 1).map((v) => (
              <button key={v} role="radio" aria-checked={rpe === v} className={rpe === v ? "tap on" : "tap"} onClick={() => setRpe(v)}>
                {v}
              </button>
            ))}
          </div>
          <div className="rpe-words" aria-hidden>
            {Array.from({ length: 10 }, (_, i) => (
              <span key={i}>{RPE_WORDS[i + 1] ?? ""}</span>
            ))}
          </div>
        </div>
        <div>
          <h2>Was the coach right?</h2>
          {nudges.length === 0 && <p className="muted">The coach made no calls this ride.</p>}
          <ol className="calls">
            {nudges.map((x, i) => (
              <li key={x.callId}>
                <div className="call-row">
                  <span className="call-no">{i + 1}</span>
                  <button className="call-v" aria-expanded={open === x.callId} onClick={() => setOpen(open === x.callId ? null : x.callId)}>
                    <span className="call-t">{mmss(x.t)}</span>{" "}
                    <b>
                      {x.verdict.replace("_", " ")} {x.proposedPctChange ? `${signed(x.proposedPctChange)} %` : ""}
                    </b>{" "}
                    <span className="muted">
                      {coachReason(x)} · {x.band} · {reverted.has(x.callId) ? "undone" : x.status === "auto_applied" ? "applied" : x.status}
                    </span>{" "}
                    <span aria-hidden>{open === x.callId ? "▾" : "▸"}</span>
                  </button>
                  <span className="call-rate">
                    {(["right", "wrong", "unsure"] as const).map((r) => (
                      <button key={r} className={rated[x.callId] === r ? "tap on" : "tap"} onClick={() => rate(x.callId, r)}>
                        {r}
                      </button>
                    ))}
                  </span>
                </div>
                {open === x.callId && <CallContext streams={streams} n={x} no={i + 1} />}
              </li>
            ))}
          </ol>
        </div>
      </section>

      <section className="card" aria-label="The ride">
        <div className="card-head">
          <h2>The ride</h2>
          <span className="legend">
            <span className="key series-power" /> power <span className="key area-key" /> planned target <span className="key series-hr" /> heart rate <span className="key series-cadence" /> cadence <span className="key marker-key" /> coach call (numbered) <span className="key gap-key" /> no data
          </span>
          <button onClick={() => setTable(!table)}>{table ? "Show chart" : "Show as table"}</button>
        </div>
        {table ? (
          <RideTable streams={streams} />
        ) : (
          <>
            <Panel n={n} unit="W" height={130} markers={markers} series={[{ values: series.target_power ?? [], cls: "target", label: "planned target", area: true }, { values: series.power ?? [], cls: "series-power", label: "power" }]} />
            <Panel n={n} unit="bpm" height={80} markers={markers} series={[{ values: series.hr ?? [], cls: "series-hr", label: "heart rate" }]} />
            <Panel n={n} unit="rpm" height={60} markers={markers} series={[{ values: series.cadence ?? [], cls: "series-cadence", label: "cadence" }]} />
            <TimeAxis n={n} />
          </>
        )}
      </section>

      <div className="two">
        <section className="card" aria-label="HR recoveries">
          <h2>HR recoveries</h2>
          <table>
            <thead>
              <tr>
                <th>#</th>
                <th>τ</th>
                <th>usual τ</th>
                <th>fit from</th>
                <th>took</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {recoveries.map((r, i) => {
                const took = (r.observed.at(-1)?.t ?? r.startedT) - r.startedT;
                return (
                  <tr key={i}>
                    <td>{i + 1}</td>
                    <td>{num(r.fit?.tauS)} s</td>
                    <td>{num(r.fit?.usualTauS)} s</td>
                    <td>{mmss(took - (r.peakT - r.startedT))} of data</td>
                    <td>{mmss(took)}</td>
                    <td>{tauSlower(r) ? <span className="status-warning">⚠ slower</span> : <span className="muted">usual</span>}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </section>
        <section className="card" aria-label="Work intervals">
          <h2>Work intervals</h2>
          <table>
            <thead>
              <tr>
                <th>interval</th>
                <th>target</th>
                <th>avg</th>
                <th>HR</th>
                <th>rpm</th>
                <th>on target</th>
              </tr>
            </thead>
            <tbody>
              {laps.map((l) => (
                <tr key={l.label}>
                  <td>{l.label}</td>
                  <td>{l.targetW} W</td>
                  <td>{l.avgW} W</td>
                  <td>{l.avgHr}</td>
                  <td>{l.cadence}</td>
                  <td>{l.compliancePct} %</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      </div>

      <section className="card all-numbers" aria-label="All numbers">
        <button className="disclosure" aria-expanded={allNumbers} onClick={() => setAllNumbers(!allNumbers)}>
          {allNumbers ? "▾" : "▸"} All numbers
        </button>
        {allNumbers && (
          <div className="tiles">
            {TILES.filter(([m]) => metricValid(m, current.compiled)).map(([m, l, u, d]) => (
              <Tile key={m} metric={m} label={l} unit={u} digits={d} />
            ))}
          </div>
        )}
      </section>

      <div className="two">
        <section className="card" aria-label="Time at power">
          <h2>Time at power</h2>
          <Bars bins={hist.bins} binWidth={hist.binWidth} />
        </section>
        <section className="card" aria-label="Power-duration">
          <h2>Power–duration</h2>
          <PdCurve mmp={mmp} height={140} />
          <p className="legend">
            <span className="key series-power" /> this ride <span className="key series-muted" /> best 90 days <span className="key series-muted dashed" /> all time
          </p>
        </section>
      </div>
      <div className="actions">
        <button className="primary" onClick={() => navigate("today")}>
          Done
        </button>
      </div>
    </div>
  );
}

export function History() {
  const s = useSession();
  const status = useStore(s?.status ?? noStatus);
  const mmp = usePoll(0.2, () => backend.mmp(), []);
  return (
    <div className="page history">
      <header className="page-head">
        <h1>History</h1>
        <p className="muted">Your model, how it got there, and every ride.</p>
      </header>
      <div className="two">
        <section className="card" aria-label="Athlete model">
          <h2>
            Athlete model <span className="badge">{calibration.confidence} confidence</span>
          </h2>
          <table>
            <tbody>
              {athleteModel.map((e) => (
                <tr key={e.label}>
                  <td>{e.label}</td>
                  <td className="num">
                    <b>{e.value}</b> ± {e.sd} {e.unit}
                  </td>
                  <td className="muted">
                    {e.source === "prior" ? "prior · calibrating" : `${e.source} · n = ${e.n}`} · {e.updated}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <h3>Critical power by week</h3>
          <Trend points={cpTrend.map((c) => ({ label: c.week, v: c.cp, sd: c.sd }))} unit="W" height={90} />
          <p className="legend">
            <span className="key series-power" /> estimate <span className="key band-key" /> ± 1 sd (narrows as rides add evidence)
          </p>
        </section>
        <section className="card" aria-label="Power-duration">
          <h2>Power–duration</h2>
          <PdCurve mmp={mmp} height={200} />
          <p className="legend">
            <span className="key series-power" /> latest ride <span className="key series-muted" /> best 90 days <span className="key series-muted dashed" /> all time
          </p>
        </section>
      </div>
      <section className="card" aria-label="Rides">
        <h2>Rides</h2>
        <ul className="rides">
          {s && status?.state === "finished" && (
            <li>
              <button className="ride-row" onClick={() => navigate("review")}>
                <span>Today</span>
                <b>{s.workout.name}</b>
                <span className="mini">
                  <Profile blocks={backend.compileWorkout(s.workout).blocks} zones={backend.zones.power} height={28} label="profile" />
                </span>
                <span>{mmss(status.t)}</span>
                <span className="muted">open review →</span>
              </button>
            </li>
          )}
          {pastRides.map((r) => {
            const parsed = backend.parseDsl(r.dsl, backend.athlete.ftp);
            const blocks = "error" in parsed ? [] : backend.compileWorkout({ id: r.id, name: r.name, ftp: backend.athlete.ftp, nodes: parsed.nodes }).blocks;
            return (
              <li key={r.id}>
                <div className="ride-row">
                  <span>{r.date}</span>
                  <b>{r.name}</b>
                  <span className="mini">
                    <Profile blocks={blocks} zones={backend.zones.power} height={28} label="profile" />
                  </span>
                  <span>
                    {mmss(r.seconds)} · TSS {r.tss}
                  </span>
                  <span className="muted">RPE {r.rpe}</span>
                </div>
              </li>
            );
          })}
        </ul>
      </section>
    </div>
  );
}
