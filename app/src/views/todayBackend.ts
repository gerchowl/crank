// The core's commands Today calls behind `mockIPC` (`rides_list`, `device_links`), for the Today tests and the design
// fixtures (app/e2e/design/fixture.tsx, #457). Today reads nothing else: a ride's load is Workouts' (rideLoad.ts), so a
// test or fixture with a load passes the workouts backend as `others`; without it, any other command is Tauri's
// plain-string rejection.
//
// Every refusal the core can give (crates/crank-core/src/core.rs):
// - `rides_list`: `storage` when the store's read fails, core.rs `Core::rides_list` (~2570) (`store.rides()?`); a read
//   maps to nothing else (error.rs `ApiError` (~54): `LockBusy` and `InvalidInput` come from opening and writing). The
//   rides come newest first (historyBackend `newestFirst`).
// - `device_links`: `storage` the same way, core.rs `Core::device_links` (~1417) (`store.devices()?`); it has no other
//   refusal, and none while a ride is loaded (it is not behind core.rs `Core::require_no_ride`). Each role is listed
//   once, trainer then strap, `device: null` when nothing is paired for it, and `link` is the loaded ride's link,
//   `null` without a ride or once it finished (core.rs `Core::device_links`). `phase` is the link keeper's phase of a
//   paired device, `riding` while the loaded ride holds it, `null` when none is paired (keeper/run.rs `Handle::links`
//   via core.rs `Core::device_links`); the mock gives a paired device without a link `cold`, as at launch.
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  DeviceLinkInfo,
  DeviceRole,
  LinkPhase,
  LinkStatus,
  PairedDevice,
  RideRecord,
} from "../bindings";
import { newestFirst, RIDES } from "./historyBackend";

export interface Backend {
  calls: { cmd: string; args: Record<string, unknown> }[];
  /** The stored rides, in any order. */
  rides: RideRecord[];
  /** Who is paired for each role. */
  paired: Partial<Record<DeviceRole, PairedDevice>>;
  /** The loaded ride's link of each role; empty without a loaded ride. */
  links: Partial<Record<DeviceRole, LinkStatus>>;
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
    paired: {},
    links: {},
    fail: {},
    hold: {},
    others: null,
    ...over,
  };
  const phaseOf = (role: DeviceRole): LinkPhase | null =>
    b.paired[role] === undefined ? null : b.links[role] ? "riding" : "cold";
  const answer = (cmd: string) => {
    switch (cmd) {
      case "rides_list":
        return [...b.rides].sort(newestFirst);
      case "device_links":
        return (["trainer", "heart_rate"] as const).map(
          (role): DeviceLinkInfo => ({
            role,
            device: b.paired[role] ?? null,
            link: b.links[role] ?? null,
            phase: phaseOf(role),
          }),
        );
      default:
        throw `command ${cmd} not found`;
    }
  };
  mockIPC((cmd, payload) => {
    if (b.others && cmd !== "rides_list" && cmd !== "device_links")
      return b.others(cmd, payload);
    b.calls.push({ cmd, args: (payload ?? {}) as Record<string, unknown> });
    if (cmd in b.fail) throw b.fail[cmd];
    const held = b.hold[cmd];
    if (held) return held.then(() => answer(cmd));
    return answer(cmd);
  });
  return b;
}
