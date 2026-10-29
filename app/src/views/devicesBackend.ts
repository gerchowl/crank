// A stand-in for the core's device commands behind `mockIPC`, for the Devices tests only. It enforces what the core
// does (crank-core `Core::devices_scan` / `device_pair` / `device_forget` / `device_links`), not more permissive:
// every one of them is `conflict` while a ride is loaded (`require_no_ride`, core.rs:535-545); pair refuses an empty
// id (`invalid_input`, core.rs:608), and, on the SIM core only (`core: "sim"`), any id but the sim's fixed one for the
// role, `trainer` / `strap` (`invalid_input`; `Backend::accepts`, backend.rs:151, `SIM_TRAINER` / `SIM_STRAP`). The
// RADIO core (the default) accepts any id and role and fails at connect (`device_unavailable`), so the UI must not
// rely on the sim's early refusal. Pair replaces the role's earlier pairing, and answers the adapter's failures;
// forget of an empty role is `not_found` (core.rs:679-696); a scan lists a peripheral once and marks `paired` for any
// role. `storage` (`store`): scan and forget read the store first (core.rs:559-566, 684-690), so they fail before
// anything else happens; pair writes it after the connect succeeded (core.rs:659), so a failure leaves nothing paired.
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  ApiError,
  DeviceLinkInfo,
  DeviceRole,
  PairedDevice,
  ScannedDevice,
} from "../bindings";

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

/** The sim core's only devices (`SIM_TRAINER`, `SIM_STRAP`, backend.rs:74-75). */
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
  /** A failure of `device_links` (the store); `null` is healthy. */
  linksError: ApiError | null;
  /** A failure of the store behind `devices_scan`, `device_pair` and `device_forget`; `null` is healthy. */
  store: ApiError | null;
  /** When set, `devices_scan`, `device_pair` and `device_forget` wait for it before answering (a command in flight). */
  gate: Promise<void> | null;
  /** Which core answers: the seeded simulator or the Bluetooth radio (default). */
  core: "sim" | "radio";
  paired: Partial<Record<DeviceRole, PairedDevice>>;
  calls: { cmd: string; args: Record<string, unknown> }[];
}

export function installDevices(init: Partial<DevicesState> = {}): DevicesState {
  const st: DevicesState = {
    nearby: [],
    adapter: null,
    connect: null,
    rideLoaded: false,
    linksError: null,
    store: null,
    gate: null,
    core: "radio",
    paired: {},
    calls: [],
    ...init,
  };
  const fail = (code: ApiError["code"], message: string): never => {
    throw { code, message } satisfies ApiError;
  };
  const noRide = () => {
    if (st.rideLoaded) fail("conflict", "a ride is loaded: end it first");
  };
  mockIPC(async (cmd, args) => {
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
        return p;
      }
      case "device_forget": {
        noRide();
        if (st.store) throw st.store;
        await st.gate;
        const role = a.role as DeviceRole;
        if (!st.paired[role]) fail("not_found", "nothing paired for that role");
        delete st.paired[role];
        return null;
      }
      case "device_links":
        if (st.linksError) throw st.linksError;
        return (["trainer", "heart_rate"] as const).map(
          (role): DeviceLinkInfo => ({
            role,
            device: st.paired[role] ?? null,
            link: null,
          }),
        );
    }
    return fail("internal", `unmocked command ${cmd}`);
  });
  return st;
}
