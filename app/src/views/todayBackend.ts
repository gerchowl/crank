// The core's command Today calls behind `mockIPC` (`rides_list`), for the Today tests and the design fixtures
// (app/e2e/design/fixture.tsx, #457). Today's devices are the snapshot channel's (`devices`, stream/uiBackend.ts) and a
// ride's load is Workouts' (rideLoad.ts), so a test or fixture passes those backends, installed first, as `others`;
// without it, any other command is Tauri's plain-string rejection.
//
// Every refusal the core can give (crates/crank-core/src/core.rs):
// - `rides_list`: `storage` when the store's read fails, core.rs `Core::rides_list` (~2570) (`store.rides()?`); a read
//   maps to nothing else (error.rs `ApiError` (~54): `LockBusy` and `InvalidInput` come from opening and writing). The
//   rides come newest first (historyBackend `newestFirst`).
import { mockIPC } from "@tauri-apps/api/mocks";
import type { RideRecord } from "../bindings";
import { newestFirst, RIDES } from "./historyBackend";

export interface Backend {
  calls: { cmd: string; args: Record<string, unknown> }[];
  /** The stored rides, in any order. */
  rides: RideRecord[];
  /** Per-command failures, thrown when the call arrives. */
  fail: Record<string, unknown>;
  /** Answers held until released, by command. */
  hold: Record<string, Promise<void>>;
  /** Answers every other command (a backend installed before this one); `null`: Tauri's rejection. */
  others: ((cmd: string, args: unknown) => Promise<unknown>) | null;
}

export function install(over: Partial<Backend> = {}): Backend {
  const b: Backend = {
    calls: [],
    rides: RIDES,
    fail: {},
    hold: {},
    others: null,
    ...over,
  };
  const answer = (cmd: string) => {
    switch (cmd) {
      case "rides_list":
        return [...b.rides].sort(newestFirst);
      default:
        throw `command ${cmd} not found`;
    }
  };
  mockIPC((cmd, payload) => {
    if (b.others && cmd !== "rides_list") return b.others(cmd, payload);
    b.calls.push({ cmd, args: (payload ?? {}) as Record<string, unknown> });
    if (cmd in b.fail) throw b.fail[cmd];
    const held = b.hold[cmd];
    if (held) return held.then(() => answer(cmd));
    return answer(cmd);
  });
  return b;
}
