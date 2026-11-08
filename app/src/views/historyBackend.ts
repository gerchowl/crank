// The core's ride-record commands behind `mockIPC` (`rides_list`, `ride_summary`), for the History tests and the
// design fixtures (app/e2e/design/fixture.tsx, #457).
import { mockIPC } from "@tauri-apps/api/mocks";
import type { RideRecord } from "../bindings";
export const apiError = (code: string, message: string) => ({ code, message });

export const RIDES: RideRecord[] = [
  {
    id: "ride-1",
    startedAt: 1_760_000_000,
    endedAt: 1_760_003_100,
    outcome: "finished",
    workoutId: "w-ss",
    workoutName: "Sweet spot 3x12",
    summary: {
      durationS: 3000,
      avgPowerW: 187.4,
      npW: 201.6,
      avgHrBpm: 142.2,
      kj: 540,
      blocks: [
        { index: 0, plannedS: 600, plannedW: 150, actualS: 600, actualW: 148 },
        { index: 1, plannedS: 720, plannedW: 230, actualS: 700, actualW: null },
      ],
    },
  },
  {
    id: "ride-3",
    startedAt: 1_758_000_000,
    endedAt: 1_758_000_300,
    outcome: "finished",
    workoutId: null,
    workoutName: "Spin",
    summary: {
      durationS: 300,
      avgPowerW: 2,
      npW: 2,
      avgHrBpm: null,
      kj: 0.6,
      blocks: [],
    },
  },
  {
    id: "ride-2",
    startedAt: 1_759_000_000,
    endedAt: 1_759_000_900,
    outcome: "aborted",
    workoutId: null,
    workoutName: "Free ride",
    summary: null,
  },
];

export interface Backend {
  calls: { cmd: string; args: Record<string, unknown> }[];
  /** The stored rides, in any order: `rides_list` answers them newest first, as the core does. */
  rides: RideRecord[];
  /** Per-command failures, thrown when the call arrives. */
  fail: Record<string, unknown>;
  /** Answers held until released, keyed by command or \`ride_summary:<id>\`. */
  hold: Record<string, Promise<void>>;
}

/**
 * A string as Rust's `{:?}` prints it (`str::escape_debug`): in quotes, with `"` and `\\` escaped, `\t` `\r` `\n`
 * `\0` by name and every other control character as `\u{hex}`. Rust escapes the rest of what it deems unprintable
 * (format characters, unassigned code points) the same way; that table is not reproduced here, so an id carrying one
 * reads unescaped in the mock.
 */
export function rustDebug(s: string): string {
  const named: Record<string, string> = {
    '"': '\\"',
    "\\": "\\\\",
    "\t": "\\t",
    "\r": "\\r",
    "\n": "\\n",
    "\0": "\\0",
  };
  return `"${s.replace(/["\\\p{Cc}]/gu, (c) => named[c] ?? `\\u{${(c.codePointAt(0) ?? 0).toString(16)}}`)}"`;
}

/** The core's order: `ORDER BY started_at DESC, id DESC`, a ride with no start last (SQLite sorts NULL lowest). */
const newestFirst = (a: RideRecord, b: RideRecord) =>
  (b.startedAt ?? -Infinity) - (a.startedAt ?? -Infinity) ||
  (a.id < b.id ? 1 : a.id > b.id ? -1 : 0);

/**
 * `rides_list` and `ride_summary` behind `mockIPC`, with every refusal the core can give
 * (crates/crank-core/src/core.rs):
 * - `rides_list`: `storage` when the store fails, core.rs:2570 `Core::rides_list` (`store.rides()?`; the store's
 *   read errors all map to `storage`, error.rs `ApiError` (~54)). It has no other refusal.
 * - `ride_summary`: core.rs:2583 `Core::ride_summary` is `rides_list` plus a find, so a store failure of `rides_list`
 *   fails it too (set `fail.rides_list`), and an id that is not stored is `not_found` `no ride "<id>"`, the id as
 *   Rust's `{id:?}` prints it (core.rs:2588, `rustDebug`). `fail.ride_summary` fails it alone (a failure between the
 *   two calls).
 * A stored summary the core cannot parse comes back as `summary: null` (core.rs:2544 `Core::record`), like an open
 * or aborted ride; both are in `RIDES`. Tauri rejects a `ride_summary` without `id` with a plain string.
 */
export function install(over: Partial<Backend> = {}): Backend {
  const b: Backend = { calls: [], rides: RIDES, fail: {}, hold: {}, ...over };
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    b.calls.push({ cmd, args });
    if (cmd in b.fail) throw b.fail[cmd];
    if (cmd === "ride_summary" && "rides_list" in b.fail)
      throw b.fail.rides_list;
    const held = b.hold[cmd] ?? b.hold[`${cmd}:${String(args.id)}`];
    if (held) return held.then(() => answer(cmd, args));
    return answer(cmd, args);
  });
  const answer = (cmd: string, args: Record<string, unknown>) => {
    switch (cmd) {
      case "rides_list":
        return [...b.rides].sort(newestFirst);
      case "ride_summary": {
        if (!("id" in args))
          throw "missing required key id for command `ride_summary`";
        const ride = b.rides.find((r) => r.id === args.id);
        if (!ride)
          throw apiError("not_found", `no ride ${rustDebug(String(args.id))}`);
        return ride;
      }
      default:
        throw `command ${cmd} not found`;
    }
  };
  return b;
}
