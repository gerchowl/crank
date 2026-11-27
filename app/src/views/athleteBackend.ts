// The core's athlete-model commands behind `mockIPC` (ADR-0009 §2.1.3), for the Settings → Athlete model tests and
// the design fixtures. It keeps the store's append-only rows (crank-store athlete.rs `append`) and resolves the model
// as the domain does (crank-domain snapshot.rs `AthleteSnapshot::resolve`: a parameter's newest row by time then id,
// else its prior, else unset), so a test sees what the real core would answer, and accepts nothing the core refuses:
//
//   athlete_model    a failed store read            storage        core.rs `Core::athlete_model` (`store`)
//   athlete_set      unknown param                  invalid_input  core.rs `athlete_param` (~1106)
//                    non-finite value or sd         invalid_input  athlete/mod.rs `check_value` (`NonFinite`)
//                    outside the param's bounds     invalid_input  athlete/mod.rs `AthleteParam::bounds`, `check_value`
//                    sd not above 0, or above 50 %  invalid_input  athlete/mod.rs `check_value` (`SdOutOfRange`)
//                    LTHR at or above a measured HR max (and an HR max at or below a measured LTHR)
//                                                   invalid_input  athlete/mod.rs `check_estimate`
//                    (`sd` omitted: 20 % of the value, athlete/prior.rs `SelfReportOrigin::Guess`)
//   athlete_history  unknown param                  invalid_input  core.rs `Core::athlete_history`
//   athlete_revert   unknown row                    not_found      athlete.rs `rows` (`Revert`)
//                    a cleared row, a prior row     invalid_input  athlete.rs `rows` (`Revert`)
//                    the copy the model refuses (an LTHR at or above a measured HR max)
//                                                   invalid_input  athlete/mod.rs `check_entry`
//   athlete_clear    unknown param                  invalid_input  core.rs `Core::athlete_clear`
//                    (appends the prior's row, HR max's from the age; else a cleared row: athlete/mod.rs `Entry::clear`)
//   any of them      a failed store write or read   storage        (`store`)
//
// Set, revert and clear supersede the parameter's open proposals (athlete.rs `append`); the proposals' own commands
// are #720's. Every command answers `null` but the two reads: the page reads the model again (`athlete_model`). A
// call without a required argument is Tauri's plain-string rejection, not an `ApiError`.
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  ApiError,
  AthleteModel,
  AthleteParam,
  AthleteProposalView,
  EstimateView,
  HistoryEntry,
  ResolvedView,
  Source,
} from "../bindings";

/** ADR-0009 §2.1.1's order (crank-domain athlete/mod.rs `AthleteParam::ALL`). */
export const PARAMS: readonly AthleteParam[] = [
  "ftp",
  "cp",
  "w_prime",
  "pmax",
  "hr_max",
  "lthr",
  "hr_rest",
  "weight",
];

/** athlete/mod.rs `AthleteParam::bounds`, inclusive. */
const BOUNDS: Record<AthleteParam, [number, number]> = {
  ftp: [50, 600],
  cp: [50, 600],
  w_prime: [2000, 60000],
  pmax: [200, 2500],
  hr_max: [120, 230],
  lthr: [100, 220],
  hr_rest: [30, 110],
  weight: [30, 200],
};

/** athlete/prior.rs `prior`: FTP and CP 250 W ± 25 %, W′ 20 kJ ± 30 %, HR max 208 − 0.7 × age ± 10 bpm. */
function prior(
  param: AthleteParam,
  age: number | null,
): { value: number; sd: number } | null {
  switch (param) {
    case "ftp":
    case "cp":
      return { value: 250, sd: 62.5 };
    case "w_prime":
      return { value: 20000, sd: 6000 };
    case "hr_max": {
      if (age === null) return null;
      const value = 208 - 0.7 * age;
      return value >= 120 && value <= 230 ? { value, sd: 10 } : null;
    }
    default:
      return null;
  }
}

/** A stored row (athlete.rs): a value, or a Clear of a parameter with no prior. */
export type Row = HistoryEntry & { param: AthleteParam };

export interface AthleteState {
  /** The rows, in any order; ids are unique. */
  rows: Row[];
  /** `athlete_profile.age`, for HR max's prior. */
  age: number | null;
  /** The open proposals (#720 raises them). */
  proposals: AthleteProposalView[];
  /** A failure of the store behind every command; `null` is healthy. */
  store: ApiError | null;
  /** The clock of the rows the commands append, UTC seconds. */
  now: number;
}

export interface Backend {
  state: AthleteState;
  calls: { cmd: string; args: Record<string, unknown> }[];
  /** Answers held until released, by command. */
  hold: Record<string, Promise<void>>;
}

const err = (code: ApiError["code"], message: string): ApiError => ({
  code,
  message,
});
const invalid = (message: string) => err("invalid_input", message);

/** A value row (`EstimateView`) as the store writes it. */
export function estimate(
  id: number,
  param: AthleteParam,
  value: number,
  over: Partial<EstimateView> = {},
): Row {
  return {
    kind: "estimate",
    id,
    param,
    value,
    sd: value * 0.2,
    source: "self_report",
    method: "manual",
    rideId: null,
    proposalId: null,
    n: 1,
    updatedAt: 0,
    ...over,
  };
}

const newestFirst = (a: Row, b: Row) =>
  (b.updatedAt ?? 0) - (a.updatedAt ?? 0) || b.id - a.id;

/** snapshot.rs `AthleteSnapshot::resolve` for one parameter, with the row it came from. */
function resolve(
  s: AthleteState,
  param: AthleteParam,
): { resolved: ResolvedView | null; estimate: EstimateView | null } {
  const newest = s.rows.filter((r) => r.param === param).sort(newestFirst)[0];
  if (newest?.kind === "estimate") {
    const row: EstimateView = newest;
    return {
      resolved: {
        value: row.value,
        sd: row.sd,
        source: row.source,
        estimateId: row.id,
      },
      estimate: row,
    };
  }
  const p = prior(param, s.age);
  return {
    resolved: p && { ...p, source: "prior", estimateId: null },
    estimate: null,
  };
}

/** A measured (non-prior) value of `param` now (snapshot.rs `Resolved::is_measured`). */
function measured(s: AthleteState, param: AthleteParam): number | null {
  const r = resolve(s, param).resolved;
  return r && r.source !== "prior" ? r.value : null;
}

/** athlete/mod.rs `check_estimate`. */
function check(
  s: AthleteState,
  param: AthleteParam,
  value: number,
  sd: number,
  source: Source,
): void {
  if (!Number.isFinite(value) || !Number.isFinite(sd))
    throw invalid(`${param}: value and sd must be finite`);
  const [min, max] = BOUNDS[param];
  if (value < min || value > max)
    throw invalid(`${param}: value must be within ${min}–${max}`);
  if (sd <= 0 || sd > 0.5 * value)
    throw invalid(`${param}: sd must be above 0 and at most 50 % of the value`);
  if (source === "prior") return;
  const pair: [number, number] | null =
    param === "lthr"
      ? (() => {
          const m = measured(s, "hr_max");
          return m === null ? null : [value, m];
        })()
      : param === "hr_max"
        ? (() => {
            const l = measured(s, "lthr");
            return l === null ? null : [l, value];
          })()
        : null;
  if (pair && pair[0] >= pair[1])
    throw invalid(`LTHR ${pair[0]} must be below HR max ${pair[1]}`);
}

const paramOf = (name: unknown): AthleteParam => {
  if (typeof name !== "string") throw "missing required key param";
  if (!(PARAMS as readonly string[]).includes(name))
    throw invalid(`unknown athlete parameter: ${JSON.stringify(name)}`);
  return name as AthleteParam;
};

/**
 * Installs the backend over whatever is installed already (the snapshot channel, stream/uiBackend.ts), which answers
 * every other command; with nothing before it, another command is Tauri's plain-string rejection.
 */
export function install(over: Partial<AthleteState> = {}): Backend {
  const state: AthleteState = {
    rows: [],
    age: null,
    proposals: [],
    store: null,
    now: 0,
    ...over,
  };
  const b: Backend = { state, calls: [], hold: {} };
  /** athlete.rs `append`: one row at `now` (or its parameter's newest time), superseding the param's proposals. */
  const append = (row: Row) => {
    const at = state.rows
      .filter((r) => r.param === row.param)
      .reduce((t, r) => Math.max(t, r.updatedAt ?? 0), state.now);
    const id = state.rows.reduce((m, r) => Math.max(m, r.id), 0) + 1;
    state.rows.push({ ...row, id, updatedAt: at });
    state.proposals = state.proposals.filter(
      (p) => !p.values.some((v) => v.param === row.param),
    );
  };
  const answer = (cmd: string, args: Record<string, unknown>): unknown => {
    switch (cmd) {
      case "athlete_model": {
        if (state.store) throw state.store;
        const model: AthleteModel = {
          params: PARAMS.map((param) => ({ param, ...resolve(state, param) })),
          proposals: state.proposals,
        };
        return model;
      }
      case "athlete_set": {
        const param = paramOf(args.param);
        if (typeof args.value !== "number") throw "missing required key value";
        const value = args.value;
        const sd = typeof args.sd === "number" ? args.sd : value * 0.2;
        if (state.store) throw state.store;
        check(state, param, value, sd, "self_report");
        append(estimate(0, param, value, { sd }));
        return null;
      }
      case "athlete_history": {
        const param = paramOf(args.param);
        if (state.store) throw state.store;
        return state.rows
          .filter((r) => r.param === param)
          .sort(newestFirst)
          .map(({ param: p, ...r }) =>
            r.kind === "estimate" ? { ...r, param: p } : r,
          );
      }
      case "athlete_revert": {
        if (typeof args.estimateId !== "number")
          throw "missing required key estimateId";
        if (state.store) throw state.store;
        const row = state.rows.find((r) => r.id === args.estimateId);
        if (!row)
          throw err("not_found", `not found: estimate ${args.estimateId}`);
        if (row.kind === "cleared")
          throw invalid("a cleared row has no value to revert to");
        if (row.source === "prior")
          throw invalid("a prior row is not reverted to: that is Clear");
        check(state, row.param, row.value!, row.sd!, row.source);
        append(row);
        return null;
      }
      case "athlete_clear": {
        const param = paramOf(args.param);
        if (state.store) throw state.store;
        const p = prior(param, state.age);
        append(
          p
            ? estimate(0, param, p.value, {
                sd: p.sd,
                source: "prior",
                method: "prior_v1",
                n: 0,
              })
            : { kind: "cleared", id: 0, param, updatedAt: 0 },
        );
        return null;
      }
      default:
        throw `command ${cmd} not found`;
    }
  };
  // An answer is a promise, so a refusal rejects as Tauri's `invoke` does.
  const inner = (
    window as unknown as {
      __TAURI_INTERNALS__?: {
        invoke: (cmd: string, args?: unknown) => Promise<unknown>;
      };
    }
  ).__TAURI_INTERNALS__?.invoke;
  mockIPC((cmd, payload) => {
    if (!cmd.startsWith("athlete_"))
      return inner
        ? inner(cmd, payload)
        : Promise.reject(`command ${cmd} not found`);
    const args = (payload ?? {}) as Record<string, unknown>;
    b.calls.push({ cmd, args });
    const held = b.hold[cmd] ?? Promise.resolve();
    return held.then(() => answer(cmd, args));
  });
  return b;
}
