// Review (DESIGN §5.5) and History (§5.6).
import { useState } from "react";
import type { MetricId } from "@crank/shared";
import { Panel, PdCurve, Profile } from "../charts";
import { mmss, num, signed } from "../format";
import { athleteModel, calibration, cpTrend, pastRides, tauSlower, workIntervals } from "../mock/ext";
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

function RideReview({ s }: { s: RideSession }) {
  const nudges = useStore(s.nudges);
  const reverted = useStore(s.reverted);
  const current = useStore(s.current);
  const status = useStore(s.status);
  const [rpe, setRpe] = useState<number | null>(null);
  const [rated, setRated] = useState<Record<string, string>>({});
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
  const rate = (callId: string, r: "right" | "wrong" | "unsure") => {
    backend.coach.rate(callId, r, true);
    setRated((p) => ({ ...p, [callId]: r }));
  };
  const maxBin = Math.max(1, ...hist.bins.map((b) => b.seconds));

  return (
    <div className="page review">
      <header className="page-head">
        <span className="eyebrow">{finished ? "Ride complete" : "Ride ended early"}</span>
        <h1>{current.workout.name}</h1>
        <p className="big-meta">
          {finished ? "All blocks ridden" : `Stopped in block ${(status?.blockIndex ?? 0) + 1} of ${current.compiled.blocks.length}`} ·{" "}
          {applied.length ? `${applied.length} coach change${applied.length > 1 ? "s" : ""} applied (${applied.map((a) => `${signed(a.proposedPctChange ?? 0)} %`).join(", ")})` : "no coach changes"} · {recoveries.length} HR-guided recoveries
        </p>
      </header>
      <div className="tiles">
        {TILES.map(([m, l, u, d]) => (
          <Tile key={m} metric={m} label={l} unit={u} digits={d} />
        ))}
      </div>

      <section className="card inputs" aria-label="Two quick inputs">
        <div>
          <h2>How hard was it?</h2>
          <div className="rpe" role="radiogroup" aria-label="RPE">
            {Array.from({ length: 10 }, (_, i) => i + 1).map((v) => (
              <button key={v} role="radio" aria-checked={rpe === v} className={rpe === v ? "tap on" : "tap"} onClick={() => setRpe(v)}>
                {v}
              </button>
            ))}
          </div>
          <p className="muted small">1 = very easy · 10 = maximal. Feeds your model (#29).</p>
        </div>
        <div>
          <h2>Was the coach right?</h2>
          {nudges.length === 0 && <p className="muted">The coach made no calls this ride.</p>}
          <ul className="calls">
            {nudges.map((x) => (
              <li key={x.callId}>
                <span className="call-t">{mmss(x.t)}</span>
                <span className="call-v">
                  <b>
                    {x.verdict.replace("_", " ")} {x.proposedPctChange ? `${signed(x.proposedPctChange)} %` : ""}
                  </b>{" "}
                  <span className="muted">
                    {x.band} · {reverted.has(x.callId) ? "undone" : x.status.replace("_", "-")}
                  </span>
                </span>
                <span className="call-rate">
                  {(["right", "wrong", "unsure"] as const).map((r) => (
                    <button key={r} className={rated[x.callId] === r ? "tap on" : "tap"} onClick={() => rate(x.callId, r)}>
                      {r}
                    </button>
                  ))}
                </span>
              </li>
            ))}
          </ul>
        </div>
      </section>

      <section className="card" aria-label="The ride">
        <h2>The ride</h2>
        <Panel n={n} unit="W" height={120} markers={markers} series={[{ values: series.target_power ?? [], cls: "target", dashed: true }, { values: series.power ?? [], cls: "series-power" }]} />
        <Panel n={n} unit="bpm" height={80} markers={markers} series={[{ values: series.hr ?? [], cls: "series-hr" }]} />
        <Panel n={n} unit="rpm" height={60} markers={markers} series={[{ values: series.cadence ?? [], cls: "series-cadence" }]} />
        <p className="muted small">Vertical lines: coach calls. Gaps: no data (sensor loss).</p>
      </section>

      <div className="two">
        <section className="card" aria-label="HR recoveries">
          <h2>HR recoveries</h2>
          <table>
            <thead>
              <tr>
                <th>#</th>
                <th>τ</th>
                <th>usual</th>
                <th>took</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {recoveries.map((r, i) => (
                <tr key={i}>
                  <td>{i + 1}</td>
                  <td>{num(r.fit?.tauS)} s</td>
                  <td>{num(r.fit?.usualTauS)} s</td>
                  <td>{mmss((r.observed.at(-1)?.t ?? r.startedT) - r.startedT)}</td>
                  <td>{tauSlower(r) ? <span className="status-warning">⚠ slower</span> : <span className="muted">normal</span>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
        <section className="card" aria-label="Work intervals">
          <h2>
            Work intervals <span className="muted small">(fixture until #12)</span>
          </h2>
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

      <div className="two">
        <section className="card" aria-label="Time at power">
          <h2>Time at power</h2>
          <div className="bars">
            {hist.bins.map((b) => (
              <i key={b.lo} style={{ height: `${(b.seconds / maxBin) * 100}%` }} title={`${b.lo} W: ${mmss(b.seconds)}`} />
            ))}
          </div>
          <p className="muted small">20 W bins, 0–{Math.round((hist.bins.at(-1)?.lo ?? 0) + 20)} W</p>
        </section>
        <section className="card" aria-label="Power-duration">
          <h2>Power–duration</h2>
          <PdCurve mmp={mmp} height={140} />
          <p className="muted small">
            <span className="key series-power" /> today <span className="key series-muted" /> 90 days <span className="key series-muted dashed" /> all time · 1 s → 60 min (log)
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
  const lo = Math.min(...cpTrend.map((c) => c.cp - c.sd));
  const hi = Math.max(...cpTrend.map((c) => c.cp + c.sd));
  const y = (v: number) => 80 - ((v - lo) / (hi - lo)) * 80;
  const x = (i: number) => (i / (cpTrend.length - 1)) * 1000;
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
          <h3>Critical power, last 9 weeks</h3>
          <svg viewBox="0 0 1000 80" preserveAspectRatio="none" style={{ height: 80, width: "100%" }} role="img" aria-label="CP trend with uncertainty">
            <polygon className="band" points={`${cpTrend.map((c, i) => `${x(i)},${y(c.cp + c.sd)}`).join(" ")} ${[...cpTrend].reverse().map((c, i) => `${x(cpTrend.length - 1 - i)},${y(c.cp - c.sd)}`).join(" ")}`} />
            <polyline className="line series-power" points={cpTrend.map((c, i) => `${x(i)},${y(c.cp)}`).join(" ")} vectorEffect="non-scaling-stroke" />
          </svg>
          <p className="muted small">Band = ± 1 sd. It narrows as rides add evidence (fixture until #18).</p>
        </section>
        <section className="card" aria-label="Power-duration">
          <h2>Power–duration</h2>
          <PdCurve mmp={mmp} height={200} />
          <p className="muted small">
            <span className="key series-power" /> latest ride <span className="key series-muted" /> 90 days <span className="key series-muted dashed" /> all time
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
                <div className="ride-row" title="Past-ride review needs rides.get (#29)">
                  <span>{r.date}</span>
                  <b>{r.name}</b>
                  <span className="mini">
                    <Profile blocks={blocks} zones={backend.zones.power} height={28} label="profile" />
                  </span>
                  <span>{r.minutes}′ · TSS {r.tss}</span>
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
