// A stand-in for the core's snapshot channel behind `mockIPC` (ADR-0001 §4.3), for the app's tests and the design
// fixtures. It models every refusal of the two commands, in the core's order, and accepts nothing the core refuses:
//
//   ui_open  the keeper cannot read the pairings  storage  (the store's text)  crank-core core.rs `Core::ui_open`,
//              through core.rs `Core::keeper` and core.rs `Core::pairings`, checked first
//            16 channels of other windows open    conflict "too many snapshot channels open"  ui.rs `Ui::open`,
//              ui.rs `MAX_CHANNELS`; a reopen from a window that holds one replaces it and is never refused
//            answers the next id from 1 (never reused), closes the window's older channel and sends the state at once
//   ui_ack   a channel the window does not hold (closed by a reopen, a page load or a failed send)  not_found
//              "unknown snapshot channel"  ui.rs `Ui::ack`; an ack of any `seq` but the last one sent is ignored
//
// The frame rule is the core's (ui.rs `run`): one frame in flight; once it is acked, the next goes out when the state
// changed or points are past the channel's cursors; sending moves the cursors, at most 256 points per series
// (ui.rs `CHUNK`); a load start resets them to 0, and a new channel starts at 0 (the backfill). Not modelled: the
// 100 ms spacing and the 1 s heartbeat (no clock here; `beat()` sends a heartbeat), and a send that fails. The page is
// one window: a reload is `pageLoad()`, which closes its channels as the page-load event does (ui.rs
// `Ui::close_window`). Faults the client must survive: `drop()` loses the next frame sent (Tauri's fetch path,
// ADR-0001 §4.3.4) and `stall()` stops the core sending until `resume()`.
import type { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  DeviceView,
  DevicesView,
  LinkPhase,
  LinkStatus,
  PairedDevice,
  Points,
  RideView,
  StreamStatus,
  UiFrame,
} from "../bindings";
import { status as makeStatus } from "./testBackend";

/** ui.rs `MAX_CHANNELS`. */
export const MAX_CHANNELS = 16;
/** The points per series per frame, ui.rs `CHUNK`. */
export const CHUNK = 256;

export type SeriesName = "power" | "cadence" | "hr";
const NAMES: readonly SeriesName[] = ["power", "cadence", "hr"];

/** A device role with nothing paired. */
export const UNPAIRED: DeviceView = {
  paired: null,
  phase: null,
  link: null,
  sample: null,
};
export const NO_DEVICES: DevicesView = { trainer: UNPAIRED, strap: UNPAIRED };

/**
 * `devices` with no ride (ui.rs `Ui::read`): each paired role in the keeper's phase (`cold` unless given, as at launch),
 * or, with a ride `link`, `riding` with it; `phase` is `null` exactly when nothing is paired.
 */
export function devicesOf(
  paired: { trainer?: PairedDevice | null; strap?: PairedDevice | null },
  links: { trainer?: LinkStatus | null; strap?: LinkStatus | null } = {},
  phases: { trainer?: LinkPhase; strap?: LinkPhase } = {},
): DevicesView {
  const one = (
    p: PairedDevice | null | undefined,
    link: LinkStatus | null | undefined,
    phase: LinkPhase | undefined,
  ): DeviceView =>
    p == null
      ? UNPAIRED
      : link != null
        ? { paired: p, phase: "riding", link, sample: null }
        : { paired: p, phase: phase ?? "cold", link: null, sample: null };
  return {
    trainer: one(paired.trainer, links.trainer, phases.trainer),
    strap: one(paired.strap, links.strap, phases.strap),
  };
}

/** A loaded ride's view, idle unless `over` says otherwise. */
export const rideView = (over: Partial<RideView> = {}): RideView => ({
  id: "ride-1",
  workoutId: "w1",
  status: makeStatus({ state: { kind: "idle" } }),
  live: { powerW: null, cadenceRpm: null, hrBpm: null },
  ended: null,
  engineUnresponsive: false,
  ...over,
});

/** What the core holds: the frame's state and the loaded ride's series. */
export interface UiTruth {
  ride: RideView | null;
  loading: boolean;
  devices: DevicesView;
  /** Per series, session second, workout second and value of each point. */
  series: Record<SeriesName, { t: number; w: number; v: number }[]>;
  /** The keeper's pairings cannot be read: `ui_open` answers `storage`. */
  keeperStorage: boolean;
  /** Channels other windows hold. */
  otherWindows: number;
}

interface Open {
  id: number;
  ch: Channel<UiFrame>;
  seq: number;
  acked: boolean;
  cursors: Record<SeriesName, number>;
  /** The state (JSON) of the last frame sent, to know when it changed. */
  sent: string;
}

const err = (code: string, message: string) => ({ code, message });

/** How the backend runs: `heartbeatMs` sends the core's heartbeat on the platform clock (the design fixtures). */
export interface UiOptions extends Partial<UiTruth> {
  heartbeatMs?: number;
}

export function installUi({ heartbeatMs, ...over }: UiOptions = {}) {
  const truth: UiTruth = {
    ride: null,
    loading: false,
    devices: NO_DEVICES,
    series: { power: [], cadence: [], hr: [] },
    keeperStorage: false,
    otherWindows: 0,
    ...over,
  };
  let open: Open | null = null;
  let nextId = 1;
  let dropNext = 0;
  let stalled = false;
  const frames: UiFrame[] = [];
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  const holds = new Map<string, Promise<void>[]>();

  const state = () =>
    JSON.stringify({
      ride: truth.ride,
      loading: truth.loading,
      devices: truth.devices,
    });
  const points = (c: Open, name: SeriesName): Points => {
    const all = truth.series[name];
    const from = c.cursors[name];
    const chunk = all.slice(from, from + CHUNK);
    c.cursors[name] = from + chunk.length;
    return {
      from,
      t: chunk.map((p) => p.t),
      w: chunk.map((p) => p.w),
      v: chunk.map((p) => p.v),
    };
  };
  const send = (c: Open) => {
    const json = state();
    const frame: UiFrame = {
      channel: c.id,
      seq: c.seq,
      ...(JSON.parse(json) as Pick<UiFrame, "ride" | "loading" | "devices">),
      series: {
        power: points(c, "power"),
        cadence: points(c, "cadence"),
        hr: points(c, "hr"),
      },
    };
    c.sent = json;
    c.acked = false;
    frames.push(frame);
    if (dropNext > 0) {
      dropNext -= 1;
      return;
    }
    c.ch.onmessage(frame);
  };
  const due = (c: Open) =>
    state() !== c.sent ||
    NAMES.some((n) => c.cursors[n] < truth.series[n].length);
  /** The core's run loop after a change or an ack: the next frame goes out if one is due and the last was acked. */
  const pump = () => {
    if (open === null || stalled || !open.acked || !due(open)) return;
    open.seq = (open.seq + 1) >>> 0;
    send(open);
  };

  const handle = (cmd: string, args: Record<string, unknown>): unknown => {
    switch (cmd) {
      case "ui_open": {
        if (truth.keeperStorage) throw err("storage", "database is locked");
        if (truth.otherWindows >= MAX_CHANNELS)
          throw err("conflict", "too many snapshot channels open");
        open = {
          id: nextId++,
          ch: args.channel as Channel<UiFrame>,
          seq: 0,
          acked: false,
          cursors: { power: 0, cadence: 0, hr: 0 },
          sent: "",
        };
        if (!stalled) send(open);
        return open.id;
      }
      case "ui_ack": {
        if (open === null || args.channel !== open.id)
          throw err("not_found", "unknown snapshot channel");
        if (args.seq === open.seq && !open.acked) {
          open.acked = true;
          pump();
        }
        return null;
      }
      default:
        return undefined;
    }
  };

  const ours = new Set(["ui_open", "ui_ack"]);
  const inner = (
    window as unknown as {
      __TAURI_INTERNALS__?: {
        invoke: (cmd: string, args?: unknown) => Promise<unknown>;
      };
    }
  ).__TAURI_INTERNALS__?.invoke;
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    if (!ours.has(cmd)) {
      if (inner) return inner(cmd, payload);
      // Tauri rejects an unknown command with a plain string, not an `ApiError`.
      return Promise.reject(`command ${cmd} not found`);
    }
    calls.push({ cmd, args });
    let result: unknown;
    let failure: { error: unknown } | null = null;
    try {
      result = handle(cmd, args);
    } catch (error) {
      failure = { error };
    }
    const gate = holds.get(cmd)?.shift();
    const settle = () => {
      if (failure) throw failure.error;
      return result;
    };
    return gate ? gate.then(settle) : settle();
  });

  const ui = {
    truth,
    calls,
    /** Every frame the core sent, dropped ones included. */
    frames,
    count: (c: string) => calls.filter((x) => x.cmd === c).length,
    /** The open channel's id, `null` when none. */
    channel: () => open?.id ?? null,
    /** The state changed (`truth` was edited): a frame goes out now, or once the last one is acked. */
    changed(): void {
      pump();
    },
    /** Edit the ride view (it must be loaded) and send the change. */
    setRide(over: Partial<RideView>): void {
      if (truth.ride === null) throw new Error("no ride loaded");
      truth.ride = { ...truth.ride, ...over };
      pump();
    },
    setStatus(over: Partial<StreamStatus>): void {
      if (truth.ride === null) throw new Error("no ride loaded");
      truth.ride = {
        ...truth.ride,
        status: { ...truth.ride.status, ...over },
      };
      pump();
    },
    /** A load starts: `loading`, no ride, the series dropped and every cursor back to 0 (ui.rs `Ui::load_started`). */
    loadStarted(): void {
      truth.loading = true;
      truth.ride = null;
      truth.series = { power: [], cadence: [], hr: [] };
      if (open) open.cursors = { power: 0, cadence: 0, hr: 0 };
      pump();
    },
    /** The load installed `ride` and is over (ui.rs `Ui::install`, `Ui::load_over`). */
    installed(ride: RideView): void {
      truth.ride = ride;
      truth.loading = false;
      if (open) open.cursors = { power: 0, cadence: 0, hr: 0 };
      pump();
    },
    /** The engine appended a point to `name` (only while riding, only a finite value, §4.3.3). */
    append(name: SeriesName, t: number, w: number, v: number): void {
      truth.series[name].push({ t, w, v });
      pump();
    },
    /** The 1 s heartbeat: a frame with nothing new, once the last is acked. */
    beat(): void {
      if (open === null || stalled || !open.acked) return;
      open.seq = (open.seq + 1) >>> 0;
      send(open);
    },
    /** The next `n` frames sent never reach the page. */
    drop(n = 1): void {
      dropNext += n;
    },
    /** The core stops sending (a stalled run loop) until `resume()`. */
    stall(): void {
      stalled = true;
    },
    resume(): void {
      stalled = false;
      if (open && open.sent === "") send(open);
      else pump();
    },
    /** The window's page (re)loads: the channel it holds is closed at the event (§4.3.5). */
    pageLoad(): void {
      open = null;
    },
    /** Delay the answer of the next call of `cmd` (its effect is at the call) until the returned function runs. */
    hold(cmd: string): () => void {
      let release = () => {};
      const gate = new Promise<void>((resolve) => (release = resolve));
      holds.set(cmd, [...(holds.get(cmd) ?? []), gate]);
      return release;
    },
  };
  // A frame at least every second on an open channel, once the last one is acked (ui.rs `run`).
  if (heartbeatMs !== undefined) setInterval(() => ui.beat(), heartbeatMs);
  return ui;
}

export type UiBackend = ReturnType<typeof installUi>;
