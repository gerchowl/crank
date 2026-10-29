// A stand-in for the core's device commands behind `mockIPC`, for the Devices tests only. It enforces what the core
// does (crank-core `Core::devices_scan` / `device_pair` / `device_forget` / `device_links`), not more permissive:
// every one of them is `conflict` while a ride is loaded; pair refuses an empty id and an id that cannot act in the
// role (`invalid_input`), replaces the role's earlier pairing, and answers the adapter's failures; forget of an
// empty role is `not_found`; a scan lists a peripheral once and marks `paired` for any role.
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

export interface DevicesState {
  /** What the next scan sees. */
  nearby: ScannedDevice[];
  /** The adapter's answer to scan and connect; `null` is healthy. */
  adapter: ApiError | null;
  /** A connect failure for `device_pair` (after the adapter check). */
  connect: ApiError | null;
  rideLoaded: boolean;
  paired: Partial<Record<DeviceRole, PairedDevice>>;
  calls: { cmd: string; args: Record<string, unknown> }[];
}

export function installDevices(init: Partial<DevicesState> = {}): DevicesState {
  const st: DevicesState = {
    nearby: [],
    adapter: null,
    connect: null,
    rideLoaded: false,
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
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    st.calls.push({ cmd, args: a });
    switch (cmd) {
      case "devices_scan": {
        noRide();
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
        if (!d || !(role === "trainer" ? d.trainer : d.heartRate))
          fail("invalid_input", `device "${id}" cannot be paired as ${role}`);
        if (st.adapter) throw st.adapter;
        if (st.connect) throw st.connect;
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
        const role = a.role as DeviceRole;
        if (!st.paired[role]) fail("not_found", "nothing paired for that role");
        delete st.paired[role];
        return null;
      }
      case "device_links":
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
