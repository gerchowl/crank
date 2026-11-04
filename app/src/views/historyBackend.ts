// The core's ride-record commands behind `mockIPC` (`rides_list`, `ride_summary`), for the History tests and the
// design fixtures (app/e2e/design/fixture.tsx, #457). Moved from History.test.tsx unchanged.
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
  /** Per-command failures, thrown when the call arrives. */
  fail: Record<string, unknown>;
  /** Answers held until released, keyed by command or \`ride_summary:<id>\`. */
  hold: Record<string, Promise<void>>;
}
/**
 * `rides_list` and `ride_summary` behind `mockIPC`, with every refusal the core can give
 * (crates/crank-core/src/core.rs): `storage` when the store fails (`rides_list`: `store.rides()?`, core.rs `rides_list` (~2128));
 * core.rs `Core::ride_summary` (~2141) propagates it) and `not_found` "no ride \"<id>\"" for an id that is not stored
 * (`ride_summary`). `rides_list` has no other refusal. A stored summary the core cannot parse comes back
 * as `summary: null` (core.rs `record` (~2102)), like an open or aborted ride; both are in `RIDES`.
 */
export function install(over: Partial<Backend> = {}): Backend {
  const b: Backend = { calls: [], fail: {}, hold: {}, ...over };
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    b.calls.push({ cmd, args });
    if (cmd in b.fail) throw b.fail[cmd];
    const held = b.hold[cmd] ?? b.hold[`${cmd}:${String(args.id)}`];
    if (held) return held.then(() => answer(cmd, args));
    return answer(cmd, args);
  });
  const answer = (cmd: string, args: Record<string, unknown>) => {
    switch (cmd) {
      case "rides_list":
        return RIDES;
      case "ride_summary": {
        if (!("id" in args))
          throw "missing required key id for command `ride_summary`";
        const ride = RIDES.find((r) => r.id === args.id);
        if (!ride) throw apiError("not_found", `no ride "${args.id}"`);
        return ride;
      }
      default:
        throw `command ${cmd} not found`;
    }
  };
  return b;
}
