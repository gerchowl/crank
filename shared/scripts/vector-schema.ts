/** Numeric kinds are declared from domain semantics, never inferred from values. */
export type VectorSchema = "float" | "integer" | "exact" | {
  fields?: Record<string, VectorSchema>;
  items?: VectorSchema;
  /** Float scalar or object/array results share a node across operation variants. */
  number?: "float";
  /** Typed-array JSON objects have decimal index keys. */
  indexed?: VectorSchema;
  ref?: string;
};
const f = "float", i = "integer", e = "exact";
const object = (fields: Record<string, VectorSchema>): VectorSchema => ({ fields });
const array = (items: VectorSchema): VectorSchema => ({ items });
const ref = (name: string): VectorSchema => ({ ref: name });
const floats = (...names: string[]): Record<string, VectorSchema> => Object.fromEntries(names.map((name) => [name, f]));
const sample = object({ t: f, v: f });
const athlete = object(floats("ftp", "weightKg", "restHr", "maxHr"));
const band = object(floats("mean", "lo", "hi"));
const fit = object({ ...floats("hrInf", "amp", "tauS", "usualTauS", "t0", "sigma"), n: i,
  grid: array(object(floats("tau", "a", "b", "c00", "c01", "c11", "w"))) });
const recoveryFields: Record<string, VectorSchema> = {
  ...floats("startedT", "peakT", "targetBpm", "minS", "maxS"), active: e, unreachable: e,
  observed: array(sample), fit, predicted: array(object({ t: f, ...floats("mean", "lo", "hi") })), readyInS: band,
};
const recovery = object(recoveryFields);
const status = object({ ...floats("t", "manualOffsetPct"), blockIndex: i, speed: i,
  state: e, mode: e, trainer: e, hrSensor: e, phase: e });
const nodes = array(ref("node"));
const workout = object({ id: e, name: e, ftp: f, nodes });
const zone = object({ name: e, min: f, max: f, color: e });
const definitions: Record<string, VectorSchema> = {
  node: object({ type: e, id: e, label: e, count: i, children: nodes, skipLastRest: e,
    progression: object(floats("wattsPerRep", "pctPerRep")), cadence: object(floats("lo", "hi")),
    end: object({ kind: e, onUnreachable: e, ...floats("s", "kj", "targetBpm", "minS", "maxS") }),
    target: object({ kind: e, ...floats("value", "lo", "hi", "from", "to", "bpm") }),
  }),
};
const areas: Record<string, { input: VectorSchema; output: VectorSchema }> = {
  dsl: { input: object({ text: e, ftp: f }), output: object({ nodes, printed: e,
    error: object({ message: e, offset: i, length: i }) }) },
  compile: { input: object({ workout, options: object({ athlete, usualTauS: f }), power: array(f) }),
    output: object({ blocks: array(object({ path: array(i), iteration: array(i), nodeId: e, phase: e, aiChanged: e,
      ...floats("startS", "durS", "powerFrom", "powerTo"), durRangeS: array(f) })),
    ...floats("totalS", "tss", "if", "np"), totalRangeS: array(f), timeInZoneS: array(f),
    issues: array(object({ severity: e, nodeId: e, message: e })) }) },
  filters: { input: object({ samples: array(sample), filters: array(object({ kind: e, n: i,
    ...floats("windowS", "tauS", "minCutoff", "beta") })) }), output: array(f) },
  history: { input: object({ samples: array(sample), reset: e, ...floats("spanS"), buckets: i, scale: e,
    query: object({ metric: e, now: f, spanS: f, buckets: i, scale: e }) }),
    output: { items: object(floats("tAgo", "dur", "mean", "min", "max")),
      fields: { edges: array(f), mapping: array(object(floats("u", "tAgo", "back"))) } } },
  wbal: { input: object({ ...floats("w", "power", "dt", "cp", "wPrime"), powers: array(f) }),
    output: { number: f, indexed: f } },
  hrv: { input: object({ rr: array(f) }), output: object(floats("rmssd", "dfaAlpha1")) },
  recovery: { input: object({ ...floats("t0", "startedT", "minS", "maxS", "usualTauS", "priorHrInf", "targetBpm", "now"),
    ts: array(f), ys: array(f), observed: array(sample), prior: object(floats("tauS", "tauLogSd", "hrInf", "hrInfSd")) }),
    output: object({ ...recoveryFields, prediction: band }) },
  athlete: { input: athlete, output: object({ physio: object(floats("cp", "wPrime", "hrAtZero", "hrAtFtp", "usualTauS")),
    powerZones: array(zone), hrZones: array(zone), power: array(object({ watts: f, hr: f, zone: i })),
    hr: array(object(floats("bpm", "watts"))) }) },
  distributions: { input: object({ metric: e, x: e, y: e, athlete, ...floats("binWidth", "xBin", "yBin", "scale"),
    series: array(f), power: array(f), durations: array(f), xs: array(f), ys: array(f) }),
    output: { items: f, fields: { metric: e, x: e, y: e, ...floats("binWidth", "xBin", "yBin", "xLo", "yLo"),
      cols: i, rows: i, bins: array(object(floats("lo", "seconds"))), typical: array(f), seconds: array(f) } } },
  simulator: { input: object({ workout, athlete, seed: i, ticks: i, dt: f, metrics: array(e) }),
    output: object({ published: array(object({ metric: e, t: f, v: f, reset: e })),
      snapshots: array(object({ status, recovery, features: object({ phase: e, blockIndex: i,
        ...floats("t", "blockElapsed", "power3s", "target", "cadence10s", "compliance", "decoupling", "hrDrift", "hrVsTypical", "wbalPct") }) })),
      events: array(object({ status, evaluate: f, recoveryEnded: recovery, recoveryUnreachable: recovery, nodeId: e })) }) },
  rewrite: { input: object({ nodes, intent: e, ftp: f, seed: i }), output: nodes },
};

/** Portable classification schema exported alongside the reference results. */
export const vectorSchemas = {
  schemaVersion: 1,
  definitions,
  areas: Object.fromEntries(Object.entries(areas).map(([area, { input, output }]) => [area,
    object({ schemaVersion: i, cases: array(object({ name: e, input, output })) }),
  ])),
};
