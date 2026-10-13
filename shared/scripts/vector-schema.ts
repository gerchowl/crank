/** Numeric kinds are declared from domain semantics, never inferred from values. */
export type VectorSchema = "float" | "integer" | "exact" | {
  fields?: Record<string, VectorSchema>;
  items?: VectorSchema;
  /** A scalar of this kind, for a node that also needs a modifier such as `nullable`. */
  kind?: "float" | "integer" | "exact";
  /** Float scalar or object/array results share a node across operation variants. */
  number?: "float";
  /** `null` is a legal value here (a missing sample or an absent function result); elsewhere it is rejected. */
  nullable?: true;
  /** Typed-array JSON objects have decimal index keys. */
  indexed?: VectorSchema;
  ref?: string;
};
const float = "float", integer = "integer", exact = "exact";
const object = (fields: Record<string, VectorSchema>): VectorSchema => ({ fields });
const array = (items: VectorSchema): VectorSchema => ({ items });
const ref = (name: string): VectorSchema => ({ ref: name });
const nullable = (schema: VectorSchema): VectorSchema =>
  typeof schema === "string" ? { kind: schema, nullable: true } : { ...schema, nullable: true };
const floats = (...names: string[]): Record<string, VectorSchema> => Object.fromEntries(names.map((name) => [name, float]));
// A missing sample is `null` in the reference JSON.
const sample = object({ t: float, v: nullable(float) });
const athlete = object(floats("ftp", "weightKg", "restHr", "maxHr"));
const band = object(floats("mean", "lo", "hi"));
const fit = object({ ...floats("hrInf", "amp", "tauS", "usualTauS", "t0", "sigma"), n: integer,
  grid: array(object(floats("tau", "a", "b", "c00", "c01", "c11", "w"))) });
const recoveryFields: Record<string, VectorSchema> = {
  ...floats("startedT", "peakT", "targetBpm", "minS", "maxS"), active: exact, unreachable: exact,
  observed: array(sample), fit: nullable(fit), predicted: array(object({ t: float, ...floats("mean", "lo", "hi") })), readyInS: band,
};
const recovery = object(recoveryFields);
const status = object({ ...floats("t", "manualOffsetPct"), blockIndex: integer, speed: integer,
  state: exact, mode: exact, trainer: exact, hrSensor: exact, phase: exact });
const nodes = array(ref("node"));
const workout = object({ id: exact, name: exact, ftp: float, nodes });
const zone = object({ name: exact, min: float, max: float, color: exact });
const definitions: Record<string, VectorSchema> = {
  node: object({ type: exact, id: exact, label: exact, count: integer, children: nodes, skipLastRest: exact,
    progression: object(floats("wattsPerRep", "pctPerRep")), cadence: object(floats("lo", "hi")),
    end: object({ kind: exact, onUnreachable: exact, ...floats("s", "kj", "targetBpm", "minS", "maxS") }),
    target: object({ kind: exact, ...floats("value", "lo", "hi", "from", "to", "bpm") }),
  }),
};
const areas: Record<string, { input: VectorSchema; output: VectorSchema }> = {
  dsl: { input: object({ text: exact, ftp: float }), output: object({ nodes, printed: exact,
    error: object({ message: exact, offset: integer, length: integer }) }) },
  compile: { input: object({ workout, options: object({ athlete, usualTauS: float }), power: array(float) }),
    output: object({ blocks: array(object({ path: array(integer), iteration: array(integer), nodeId: exact, phase: exact, aiChanged: exact,
      ...floats("startS", "durS", "powerFrom", "powerTo"), durRangeS: array(float) })),
    ...floats("totalS", "tss", "if", "np"), totalRangeS: array(float), timeInZoneS: array(integer),
    issues: array(object({ severity: exact, nodeId: exact, message: exact })) }) },
  filters: { input: object({ samples: array(sample), filters: array(object({ kind: exact, n: integer,
    ...floats("windowS", "tauS", "minCutoff", "beta") })) }), output: array(nullable(float)) },
  history: { input: object({ samples: array(sample), reset: exact, ...floats("spanS"), buckets: integer, scale: exact,
    query: object({ metric: exact, now: float, spanS: float, buckets: integer, scale: exact }) }),
    output: { items: object(floats("tAgo", "dur", "mean", "min", "max")),
      fields: { edges: array(float), mapping: array(object(floats("u", "tAgo", "back"))) } } },
  wbal: { input: object({ ...floats("w", "power", "dt", "cp", "wPrime"), powers: array(float) }),
    output: { number: float, indexed: float } },
  hrv: { input: object({ rr: array(float) }), output: object({ rmssd: nullable(float), dfaAlpha1: nullable(float) }) },
  recovery: { input: object({ ...floats("t0", "startedT", "minS", "maxS", "usualTauS", "priorHrInf", "targetBpm", "now"),
    ts: array(float), ys: array(float), observed: array(sample), prior: object(floats("tauS", "tauLogSd", "hrInf", "hrInfSd")) }),
    output: object({ ...recoveryFields, prediction: nullable(band) }) },
  athlete: { input: athlete, output: object({ physio: object(floats("cp", "wPrime", "hrAtZero", "hrAtFtp", "usualTauS")),
    powerZones: array(zone), hrZones: array(zone), power: array(object({ watts: float, hr: float, zone: integer })),
    hr: array(object(floats("bpm", "watts"))) }) },
  distributions: { input: object({ metric: exact, x: exact, y: exact, athlete, ...floats("binWidth", "xBin", "yBin", "scale"),
    series: array(float), power: array(float), durations: array(float), xs: array(float), ys: array(float) }),
    output: { items: float, fields: { metric: exact, x: exact, y: exact, ...floats("binWidth", "xBin", "yBin", "xLo", "yLo"),
      cols: integer, rows: integer, bins: array(object({ lo: float, seconds: integer })), typical: array(float), seconds: array(integer) } } },
  simulator: { input: object({ workout, athlete, seed: integer, ticks: integer, dt: float, metrics: array(exact) }),
    output: object({ published: array(object({ metric: exact, t: float, v: nullable(float), reset: exact })),
      snapshots: array(object({ status, recovery, features: object({ phase: exact, blockIndex: integer,
        ...floats("t", "blockElapsed", "power3s", "target", "cadence10s", "compliance", "hrVsTypical", "wbalPct"),
        decoupling: nullable(float), hrDrift: nullable(float) }) })),
      events: array(object({ status, evaluate: float, recoveryEnded: recovery, recoveryUnreachable: recovery, nodeId: exact })) }) },
  rewrite: { input: object({ nodes, intent: exact, ftp: float, seed: integer }), output: nodes },
};

/** Portable classification schema exported alongside the reference results. */
export const vectorSchemas = {
  schemaVersion: 2,
  definitions,
  areas: Object.fromEntries(Object.entries(areas).map(([area, { input, output }]) => [area,
    object({ schemaVersion: integer, cases: array(object({ name: exact, input, output })) }),
  ])),
};
