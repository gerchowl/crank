// The window's snapshot channel (ADR-0001 §4.3, #598): one `Channel<UiFrame>`, opened when the app mounts, that
// carries the whole live state. Framework-free; `uiHooks.tsx` binds it to React.
//
// The one client rule (§4.3.2): for each frame of the channel it holds, in arrival order, the client replaces `ride`,
// `loading` and `devices`, appends each series' points (or replaces the series when `from` is 0), then acks with
// `ui_ack(frame.channel, frame.seq)`. It compares no `seq`, runs no re-query, merges no command's answer, keeps nothing
// from an earlier frame but the series, and drops the frames of a channel it replaced. `from` always equals the length
// the page holds (the core's property test, `ui::tests::the_channel_holds_over_arbitrary_interleavings`): a mismatch
// is a core bug. A dev build throws on it. A production build does not repair the series: it logs the fault once,
// reports it as `internal` and shows the state as stale (every route shows that, `ChannelNotice` and the ride screen's
// band), and opens a new channel at once, whose backfill from index 0 replaces the broken series.
//
// **Watchdog (§4.3.4).** The core sends a frame at least every second on an open channel (the heartbeat), so 2 s
// without one means a frame was lost (Tauri's fetch path for a frame of 8 192 B or more ends in
// `.catch(console.error)`) or the core is not sending. The client then shows the state as stale (`stale`, which the
// ride screen shows as the engine-unresponsive banner: the values are not live) and calls `ui_open` again; the core
// closes the older channel and the new one backfills from index 0. A stall shorter than 2 s catches up without loss
// when the page acks; a longer one costs a backfill. Each reopen is logged and counted (`reopens`): if the fetch path
// failed every time, the page would reopen every 2 s with its live state frozen, and a loss mid-backfill restarts
// from 0 (#599's `since` cursor removes the restart).
//
// The watchdog is one cancellable timeout re-armed by every frame; its clock is injected (`Timers`), so tests run it
// on a fake clock. No library was found that does less than this (see the PR of #598 for the search).
import { Channel } from "@tauri-apps/api/core";
import { api } from "../api";
import {
  commands,
  type ApiError,
  type DevicesView,
  type Points,
  type RideView,
  type UiChannelId,
  type UiFrame,
} from "../bindings";

/** No frame for this long on an open channel and the client opens a new one (ADR-0001 §4.3.4). */
export const WATCHDOG_MS = 2000;

/** The live series of a ride: the three traces (DESIGN §18.3). */
export type Series = "power" | "hr" | "cadence";
export const SERIES: readonly Series[] = ["power", "cadence", "hr"];

/** One point of a series: session second `t`, workout second `workoutS`, and the value (W, bpm or rpm). */
export interface Point {
  readonly t: number;
  readonly workoutS: number;
  readonly v: number;
}

/** The clock the watchdog runs on: the platform's timers, or a fake one in tests. */
export interface Timers {
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}
const PLATFORM: Timers = {
  setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms),
  clearTimeout: (h) => globalThis.clearTimeout(h as number),
};

/** How a channel is made: Tauri's IPC channel, or a test's. */
export type MakeChannel = () => Channel<UiFrame>;

/** Everything the page shows of the core's live state. Replaced, never mutated, so a reader can hold a reference. */
export interface UiSnapshot {
  /** A frame has arrived: until then `devices` is unknown and `ride` says nothing. */
  readonly received: boolean;
  /** The loaded ride, or the last ended one (`UiFrame.ride`). */
  readonly ride: RideView | null;
  /** A `ride_load` is under way. */
  readonly loading: boolean;
  /** Both device roles; `null` until the first frame. */
  readonly devices: DevicesView | null;
  /** The loaded ride's series, as the frames built them. */
  readonly series: Readonly<Record<Series, readonly Point[]>>;
  /** No frame for 2 s: what is shown is not live until the next frame. */
  readonly stale: boolean;
  /** The channel's own failure (`ui_open`, `ui_ack`, a broken `from`); cleared by the next frame. */
  readonly error: ApiError | null;
  /** How often the watchdog has reopened the channel since the client started. */
  readonly reopens: number;
}

const NO_SERIES: UiSnapshot["series"] = { power: [], cadence: [], hr: [] };
export const EMPTY: UiSnapshot = {
  received: false,
  ride: null,
  loading: false,
  devices: null,
  series: NO_SERIES,
  stale: false,
  error: null,
  reopens: 0,
};

/**
 * Unregister a channel's IPC callback. `Channel.cleanupCallback` is what the channel runs itself when the core ends
 * it (@tauri-apps/api 2.12.3 core.js `Channel`); it is private only in the type declarations, and the core never ends
 * a snapshot channel it closes, so the client calls it when it replaces one. (The last channel goes with the page.)
 */
function release(channel: Channel<UiFrame>): void {
  (channel as unknown as { cleanupCallback(): void }).cleanupCallback();
}

/** `points` applied to `held` by the client rule; a broken `from` keeps `held` and names the fault. */
function applyPoints(
  held: readonly Point[],
  points: Points,
  name: Series,
): { series: readonly Point[]; fault: string | null } {
  if (points.from !== 0 && points.from !== held.length)
    return {
      series: held,
      fault: `${name}: from ${points.from} on a series of ${held.length}`,
    };
  if (points.from === 0 && points.v.length === 0 && held.length === 0)
    return { series: held, fault: null };
  const fresh: Point[] = [];
  for (let i = 0; i < points.v.length; i++) {
    const t = points.t[i];
    const w = points.w[i];
    // The core's `f64`s; specta types them nullable, the store never holds a non-finite one (§4.3.3).
    if (typeof t !== "number" || typeof w !== "number")
      return { series: held, fault: `${name}: point ${i} has no time` };
    fresh.push({ t, workoutS: w, v: points.v[i] });
  }
  if (points.from === 0) return { series: fresh, fault: null };
  if (fresh.length === 0) return { series: held, fault: null };
  return { series: [...held, ...fresh], fault: null };
}

/** The snapshot channel of one window. */
export class UiClient {
  private snap: UiSnapshot = EMPTY;
  private readonly listeners = new Set<() => void>();
  private retained = 0;
  /** The channel whose frames are applied; any other's are dropped. */
  private channel: Channel<UiFrame> | null = null;
  private watchdog: unknown = null;

  /**
   * `dev`: a broken `from` throws (a dev or test build); otherwise it is the production path of the header. Tests
   * pass `false` to run that path.
   */
  constructor(
    private readonly timers: Timers = PLATFORM,
    private readonly makeChannel: MakeChannel = () => new Channel<UiFrame>(),
    private readonly dev: boolean = import.meta.env.DEV,
  ) {}

  /** The current snapshot; a new object whenever it changes. */
  get snapshot(): UiSnapshot {
    return this.snap;
  }

  /** `useSyncExternalStore`'s subscribe. */
  readonly listen = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => void this.listeners.delete(listener);
  };

  private set(next: UiSnapshot): void {
    this.snap = next;
    for (const l of this.listeners) l();
  }

  /** Mark a consumer; the first one opens the channel. Call the returned function to release. */
  retain(): () => void {
    this.retained += 1;
    if (this.retained === 1 && this.channel === null) this.open();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.retained -= 1;
      // A remount in the same task (StrictMode) keeps the channel.
      if (this.retained === 0)
        queueMicrotask(() => this.retained === 0 && this.close());
    };
  }

  /**
   * Open a channel now, whatever the watchdog says (the channel notice's Retry after a refused `ui_open`): the core
   * closes the window's older one and the new one backfills from 0.
   */
  reopen(): void {
    if (this.channel !== null) this.open();
  }

  /** Open a channel; the core closes the window's older one (§4.3.5) and the new one backfills from 0. */
  private open(): void {
    // The replaced channel's IPC callback is released: its frames are dropped anyway, and nothing else frees it.
    if (this.channel) release(this.channel);
    const channel = this.makeChannel();
    channel.onmessage = (frame) => this.onFrame(channel, frame);
    this.channel = channel;
    this.arm();
    void api(commands.uiOpen(channel)).then((r) => {
      // `conflict` beyond 16 channels, `storage` when the keeper cannot read the pairings: shown, and the watchdog
      // opens again in 2 s, as for a lost frame.
      if (r.status === "error" && channel === this.channel)
        this.set({ ...this.snap, error: r.error });
    });
  }

  /** No consumer left: the channel is forgotten; the core closes it when the window opens another or reloads. */
  private close(): void {
    this.disarm();
    this.channel = null;
    this.set(EMPTY);
  }

  private arm(): void {
    this.disarm();
    this.watchdog = this.timers.setTimeout(() => this.bark(), WATCHDOG_MS);
  }

  private disarm(): void {
    if (this.watchdog !== null) this.timers.clearTimeout(this.watchdog);
    this.watchdog = null;
  }

  /**
   * 2 s without a frame: a new channel, and the state is stale if the page had any. A channel that never answered
   * (`ui_open` refused) is not stale: its refusal is what the page shows.
   */
  private bark(): void {
    this.watchdog = null;
    if (this.channel === null) return;
    const reopens = this.snap.reopens + 1;
    console.warn(
      `ui channel: no frame for ${WATCHDOG_MS} ms, reopening (${reopens} so far)`,
    );
    this.set({ ...this.snap, stale: this.snap.received, reopens });
    this.open();
  }

  private onFrame(channel: Channel<UiFrame>, frame: UiFrame): void {
    if (channel !== this.channel) return;
    this.arm();
    const held = this.snap.series;
    const series = { ...held };
    const faults: string[] = [];
    for (const name of SERIES) {
      const r = applyPoints(held[name], frame.series[name], name);
      series[name] = r.series;
      if (r.fault !== null) faults.push(r.fault);
    }
    const fault = faults.length > 0 ? faults.join("; ") : null;
    if (fault !== null && this.dev) throw new Error(`ui channel: ${fault}`);
    this.set({
      received: true,
      ride: frame.ride,
      loading: frame.loading,
      devices: frame.devices,
      series: SERIES.every((n) => series[n] === held[n])
        ? held
        : Object.freeze(series),
      stale: fault !== null,
      error: fault === null ? null : { code: "internal", message: fault },
      reopens: this.snap.reopens + (fault === null ? 0 : 1),
    });
    if (fault === null) {
      this.ack(channel, frame.channel, frame.seq);
      return;
    }
    // A core bug (ADR-0001 §4.3.2), not repaired: logged once, shown as stale, and a new channel's backfill from 0
    // replaces the series. The broken channel is not acked; the core closes it at the reopen.
    console.error(`ui channel: ${fault}; reopening`);
    this.open();
  }

  private ack(channel: Channel<UiFrame>, id: UiChannelId, seq: number): void {
    void api(commands.uiAck(id, seq)).then((r) => {
      // `not_found` is a channel the core has closed (replaced by a newer one, or after a failed send): nothing to
      // show; the watchdog reopens a channel that has gone quiet.
      if (r.status === "error" && r.error.code !== "not_found")
        if (channel === this.channel)
          this.set({ ...this.snap, error: r.error });
    });
  }
}

/** The window's client. Nothing touches the core until a `UiProvider` retains it. */
export const windowUi = new UiClient();
