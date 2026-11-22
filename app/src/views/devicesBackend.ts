// A stand-in for the core's device commands behind `mockIPC`, for the Devices tests and the design fixtures. It
// enforces what the core does (crank-core `Core::devices_scan` / `device_pair` / `device_forget`), not more permissive:
// every one of them is `conflict` while a ride is loaded (core.rs `require_no_ride` (~1104)); pair refuses an empty
// id (`invalid_input`, core.rs `device_pair` (~1170)), and, on the SIM core only (`core: "sim"`), any id but the sim's fixed one for the
// role, `trainer` / `strap` (`invalid_input`; backend.rs `Backend::accepts` (~206), `SIM_TRAINER` / `SIM_STRAP`). The
// RADIO core (the default) accepts any id and role and fails at connect (`device_unavailable`), so the UI must not
// rely on the sim's early refusal. Pair replaces the role's earlier pairing, and answers the adapter's failures;
// forget of an empty role is `not_found` (core.rs `device_forget` (~1248)); a scan lists a peripheral once and marks `paired` for any
// role. `storage` (`store`): scan and forget read the store first (core.rs `devices_scan` (~1125), `device_forget`), so they fail before
// anything else happens; pair writes it after the connect succeeded (core.rs `device_pair`): `store` then leaves nothing paired,
// `storeAfterWrite` models the later reads and forgets of the replaced pairing failing (core.rs `device_pair`, `device_forget`), after the
// new row was written. `device_pair` and `device_forget` answer `null` (app/src-tauri commands.rs `device_pair`): the
// pairings reach the page as the snapshot channel's `devices` (`ui`, stream/uiBackend.ts; core.rs `Core::device_pair`
// and core.rs `Core::device_forget` call ui.rs `Ui::set_paired`), also when the pair's later store step fails.
// `ui_open` and `ui_ack` go to that backend, installed first.
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  ApiError,
  DeviceRole,
  PairedDevice,
  ScannedDevice,
} from "../bindings";
import {
  devicesOf,
  installUi,
  type UiBackend,
  type UiOptions,
} from "../stream/uiBackend";

export const trainer: ScannedDevice = {
  id: "neo-1",
  name: "Tacx Neo 55624",
  trainer: true,
  heartRate: false,
  paired: false,
};
export const strap: ScannedDevice = {
  id: "hrm-1",
  name: "HRM-Dual",
  trainer: false,
  heartRate: true,
  paired: false,
};

/** A peripheral that offers both a trainer protocol and the Heart Rate Service. */
export const both: ScannedDevice = {
  id: "both-1",
  name: "Dual Thing",
  trainer: true,
  heartRate: true,
  paired: false,
};

/** The sim core's only devices (`SIM_TRAINER`, `SIM_STRAP`, backend.rs `SIM_TRAINER` (~128)). */
export const simTrainer: ScannedDevice = {
  id: "trainer",
  name: "Simulated trainer",
  trainer: true,
  heartRate: false,
  paired: false,
};
export const simStrap: ScannedDevice = {
  id: "strap",
  name: "Simulated strap",
  trainer: false,
  heartRate: true,
  paired: false,
};

export interface DevicesState {
  /** What the next scan sees. */
  nearby: ScannedDevice[];
  /** The adapter's answer to scan and connect; `null` is healthy. */
  adapter: ApiError | null;
  /** A connect failure for `device_pair` (after the adapter check). */
  connect: ApiError | null;
  rideLoaded: boolean;
  /** A failure of the store behind `devices_scan`, `device_pair` and `device_forget`; `null` is healthy. */
  store: ApiError | null;
  /** A failure of `device_pair` after its row was written (the replaced-pairing cleanup); `null` is healthy. */
  storeAfterWrite: ApiError | null;
  /** When set, `devices_scan`, `device_pair` and `device_forget` wait for it before answering (a command in flight). */
  gate: Promise<void> | null;
  /** When set, `device_pair` alone also waits for it (a pair in flight after a scan that answered). */
  pairGate: Promise<void> | null;
  /** Which core answers: the seeded simulator or the Bluetooth radio (default). */
  core: "sim" | "radio";
  paired: Partial<Record<DeviceRole, PairedDevice>>;
  calls: { cmd: string; args: Record<string, unknown> }[];
  /** The window's snapshot channel, which carries the pairings. */
  ui: UiBackend;
}

export function installDevices(
  init: Partial<Omit<DevicesState, "ui">> = {},
  uiOptions: UiOptions = {},
): DevicesState {
  const ui = installUi(uiOptions);
  const prev = (
    window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (cmd: string, args?: unknown) => Promise<unknown>;
      };
    }
  ).__TAURI_INTERNALS__.invoke;
  const st: DevicesState = {
    nearby: [],
    adapter: null,
    connect: null,
    rideLoaded: false,
    store: null,
    storeAfterWrite: null,
    gate: null,
    pairGate: null,
    core: "radio",
    paired: {},
    calls: [],
    ...init,
    ui,
  };
  /** The pairings changed: the channel's next frame carries them (ui.rs `Ui::set_paired`). */
  const show = () => {
    ui.truth.devices = devicesOf({
      trainer: st.paired.trainer,
      strap: st.paired.heart_rate,
    });
    ui.changed();
  };
  show();
  const fail = (code: ApiError["code"], message: string): never => {
    throw { code, message } satisfies ApiError;
  };
  const noRide = () => {
    if (st.rideLoaded) fail("conflict", "a ride is loaded: end it first");
  };
  mockIPC(async (cmd, args) => {
    if (cmd === "ui_open" || cmd === "ui_ack") return prev(cmd, args);
    const a = (args ?? {}) as Record<string, unknown>;
    st.calls.push({ cmd, args: a });
    switch (cmd) {
      case "devices_scan": {
        noRide();
        if (st.store) throw st.store;
        await st.gate;
        if (st.adapter) throw st.adapter;
        const known = new Set(Object.values(st.paired).map((p) => p.id));
        return st.nearby.map((d) => ({ ...d, paired: known.has(d.id) }));
      }
      case "device_pair": {
        noRide();
        const id = a.id as string;
        const role = a.role as DeviceRole;
        if (id === "") fail("invalid_input", "empty device id");
        const d = st.nearby.find((n) => n.id === id);
        const fits = d && (role === "trainer" ? d.trainer : d.heartRate);
        // The sim has one fixed id per role and refuses every other before it connects.
        const simId = role === "trainer" ? "trainer" : "strap";
        if (st.core === "sim" && id !== simId)
          fail("invalid_input", `device "${id}" cannot be paired as ${role}`);
        await st.gate;
        await st.pairGate;
        if (st.adapter) throw st.adapter;
        if (st.connect) throw st.connect;
        // The radio finds out at connect; the sim's device always exists.
        if (st.core === "radio" && !fits)
          fail("device_unavailable", "connect failed");
        // The store is written after the connect, so a failure leaves nothing paired.
        if (st.store) throw st.store;
        const p: PairedDevice = {
          role,
          id,
          name: d?.name ?? null,
          serial: "55624",
          model: role === "trainer" ? "Neo" : null,
        };
        st.paired[role] = p;
        show();
        if (st.storeAfterWrite) throw st.storeAfterWrite;
        return null;
      }
      case "device_forget": {
        noRide();
        if (st.store) throw st.store;
        await st.gate;
        const role = a.role as DeviceRole;
        if (!st.paired[role]) fail("not_found", "nothing paired for that role");
        delete st.paired[role];
        show();
        return null;
      }
    }
    return fail("internal", `unmocked command ${cmd}`);
  });
  return st;
}
