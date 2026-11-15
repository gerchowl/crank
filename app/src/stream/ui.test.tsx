// @vitest-environment jsdom
// The window's snapshot channel client (ADR-0001 §4.3.2, §4.3.4; #598), against the core's contract as
// stream/uiBackend.ts models it: the client rule, the acks, the refusals of `ui_open` and `ui_ack`, and the 2 s watchdog
// on an injected clock (a dropped frame, a stalled core, a page reload mid-ride).
import { act, cleanup, render, renderHook } from "@testing-library/react";
import { clearMocks } from "@tauri-apps/api/mocks";
import { StrictMode, type ReactNode } from "react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { status } from "./testBackend";
import { UiClient, WATCHDOG_MS, type Point, type Timers } from "./ui";
import { CHUNK, MAX_CHANNELS, installUi, rideView } from "./uiBackend";
import { UiProvider, useUi } from "./uiHooks";

afterEach(() => {
  cleanup();
  clearMocks();
  vi.restoreAllMocks();
});

/** A clock the test moves: `advance` runs every timeout that falls due, in order. */
class FakeClock implements Timers {
  now = 0;
  private next = 1;
  private readonly due = new Map<number, { at: number; fn: () => void }>();
  setTimeout(fn: () => void, ms: number): unknown {
    const id = this.next++;
    this.due.set(id, { at: this.now + ms, fn });
    return id;
  }
  clearTimeout(handle: unknown): void {
    this.due.delete(handle as number);
  }
  /** Move the clock by `ms`, running each timeout at its time, and let the answers it caused land. */
  async advance(ms: number): Promise<void> {
    const end = this.now + ms;
    for (;;) {
      const first = [...this.due.entries()]
        .filter(([, d]) => d.at <= end)
        .sort((a, b) => a[1].at - b[1].at)[0];
      if (!first) break;
      this.due.delete(first[0]);
      this.now = first[1].at;
      first[1].fn();
      await flush();
    }
    this.now = end;
    await flush();
  }
}

/** Let the mock's answers (promises) land. */
const flush = () =>
  act(async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  });

const setup = (over: Parameters<typeof installUi>[0] = {}) => {
  const ui = installUi(over);
  const clock = new FakeClock();
  const client = new UiClient(clock);
  const release = client.retain();
  return { ui, clock, client, release };
};

/** `n` points of a series at 4 Hz from session second `t0`, as the engine appends them. */
const appendPower = (ui: ReturnType<typeof installUi>, n: number, t0 = 0) => {
  for (let i = 0; i < n; i++) {
    const t = t0 + i / 4;
    ui.truth.series.power.push({ t, w: t, v: 200 + (i % 50) });
  }
  ui.changed();
};
const values = (points: readonly Point[]) => points.map((p) => p.v);
const coreValues = (ui: ReturnType<typeof installUi>) =>
  ui.truth.series.power.map((p) => p.v);

describe("the client rule", () => {
  test("applies every frame in order, replacing the state, and acks each with its channel and seq", async () => {
    const { ui, client } = setup({ ride: rideView() });
    await flush();
    expect(client.snapshot.received).toBe(true);
    expect(client.snapshot.ride?.id).toBe("ride-1");
    ui.setStatus({ workoutS: 12 });
    await flush();
    ui.setStatus({ workoutS: 13 });
    await flush();
    expect(client.snapshot.ride?.status.workoutS).toBe(13);
    const acks = ui.calls.filter((c) => c.cmd === "ui_ack").map((c) => c.args);
    expect(acks).toEqual([
      { channel: 1, seq: 0 },
      { channel: 1, seq: 1 },
      { channel: 1, seq: 2 },
    ]);
    // Nothing kept from an earlier frame but the series: an unload clears the ride.
    ui.truth.ride = null;
    ui.changed();
    await flush();
    expect(client.snapshot.ride).toBeNull();
  });

  test("appends the series, replaces it on from 0, and a load start empties it", async () => {
    const { ui, client } = setup({ ride: rideView() });
    await flush();
    appendPower(ui, 10);
    await flush();
    appendPower(ui, 5, 10);
    await flush();
    expect(values(client.snapshot.series.power)).toEqual(coreValues(ui));
    expect(client.snapshot.series.power).toHaveLength(15);
    ui.loadStarted();
    await flush();
    expect(client.snapshot.series.power).toEqual([]);
    expect(client.snapshot.loading).toBe(true);
    expect(client.snapshot.ride).toBeNull();
    ui.installed(rideView({ id: "ride-2" }));
    appendPower(ui, 3);
    await flush();
    expect(client.snapshot.ride?.id).toBe("ride-2");
    expect(values(client.snapshot.series.power)).toEqual(coreValues(ui));
  });

  test("a backlog arrives 256 points per series per frame, contiguous", async () => {
    const { ui, client } = setup({ ride: rideView() });
    appendPower(ui, 3 * CHUNK + 7);
    await flush();
    // The first frame went out at the open, before the points.
    expect(ui.frames.map((f) => f.series.power.v.length)).toEqual([
      0,
      CHUNK,
      CHUNK,
      CHUNK,
      7,
    ]);
    expect(values(client.snapshot.series.power)).toEqual(coreValues(ui));
  });

  test("a from that is not the page's length is a core bug: a dev build throws", async () => {
    const ui = installUi({ ride: rideView() });
    const client = new UiClient(new FakeClock());
    client.retain();
    await flush();
    // A frame the core cannot send: from 5 on an empty series.
    const channel = ui.calls.find((c) => c.cmd === "ui_open")?.args.channel as {
      onmessage: (f: unknown) => void;
    };
    expect(() =>
      channel.onmessage({
        ...ui.frames[0],
        seq: 1,
        series: {
          ...ui.frames[0].series,
          power: { from: 5, t: [1], w: [1], v: [100] },
        },
      }),
    ).toThrow(/power: from 5 on a series of 0/);
  });

  test("in production a broken from is logged once, shown stale and internal, not acked, and the watchdog's backfill repairs it", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const ui = installUi({ ride: rideView() });
    const clock = new FakeClock();
    const client = new UiClient(clock, undefined, false);
    client.retain();
    appendPower(ui, 10);
    await flush();
    const channel = ui.calls.find((c) => c.cmd === "ui_open")?.args.channel as {
      onmessage: (f: unknown) => void;
    };
    const acks = ui.count("ui_ack");
    // A frame the core cannot send: from 3 on a series of 10.
    act(() =>
      channel.onmessage({
        ...ui.frames[0],
        seq: 9,
        series: {
          ...ui.frames[0].series,
          power: { from: 3, t: [1], w: [1], v: [999] },
        },
      }),
    );
    await flush();
    expect(client.snapshot.stale).toBe(true);
    expect(client.snapshot.error).toEqual({
      code: "internal",
      message: "power: from 3 on a series of 10",
    });
    expect(error).toHaveBeenCalledTimes(1);
    expect(error).toHaveBeenCalledWith(
      "ui channel: power: from 3 on a series of 10; not acked, the watchdog reopens in 2000 ms",
    );
    // Not repaired in place, not acked, no reopen yet: the watchdog does it.
    expect(ui.count("ui_ack")).toBe(acks);
    expect(ui.count("ui_open")).toBe(1);
    expect(values(client.snapshot.series.power)).toHaveLength(10);
    await clock.advance(WATCHDOG_MS);
    expect(ui.count("ui_open")).toBe(2);
    expect(client.snapshot.stale).toBe(false);
    expect(client.snapshot.error).toBeNull();
    expect(values(client.snapshot.series.power)).toEqual(coreValues(ui));
    expect(error).toHaveBeenCalledTimes(1);
  });

  test("a fault on every backfill costs one reopen per 2 s and one log line, never a loop", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const ui = installUi({ ride: rideView() });
    // A point the core can never send (no session time): every frame that carries it, every backfill, is broken.
    appendPower(ui, 4);
    (ui.truth.series.power[0] as { t: number | null }).t = null;
    const clock = new FakeClock();
    const client = new UiClient(clock, undefined, false);
    client.retain();
    await flush();
    expect(client.snapshot.stale).toBe(true);
    expect(client.snapshot.error?.message).toBe("power: point 0 has no time");
    await clock.advance(10_000);
    // 10 s: the first open plus one reopen per 2 s.
    expect(ui.count("ui_open")).toBe(1 + 10_000 / WATCHDOG_MS);
    expect(client.snapshot.reopens).toBe(10_000 / WATCHDOG_MS);
    expect(client.snapshot.stale).toBe(true);
    expect(error).toHaveBeenCalledTimes(1);
  });

  test("a fault that comes back after a repair is logged again", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const ui = installUi({ ride: rideView() });
    const clock = new FakeClock();
    const client = new UiClient(clock, undefined, false);
    client.retain();
    appendPower(ui, 4);
    await flush();
    const broken = () => {
      const channel = ui.calls.filter((c) => c.cmd === "ui_open").at(-1)?.args
        .channel as { onmessage: (f: unknown) => void };
      const last = ui.frames.at(-1)!;
      act(() =>
        channel.onmessage({
          ...last,
          seq: last.seq + 1,
          series: {
            ...last.series,
            power: { from: 9, t: [1], w: [1], v: [1] },
          },
        }),
      );
    };
    broken();
    expect(error).toHaveBeenCalledTimes(1);
    // The watchdog's reopen repairs it: a clean frame.
    await clock.advance(WATCHDOG_MS);
    expect(client.snapshot.stale).toBe(false);
    // The same kind of fault again, after the repair: a new log line.
    broken();
    expect(client.snapshot.stale).toBe(true);
    expect(error).toHaveBeenCalledTimes(2);
  });

  test("a replaced channel's IPC callback is released", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const internals = (
      window as unknown as {
        __TAURI_INTERNALS__: { unregisterCallback: (id: number) => void };
      }
    ).__TAURI_INTERNALS__;
    const { ui, clock } = setup({ ride: rideView() });
    await flush();
    const first = ui.calls.find((c) => c.cmd === "ui_open")?.args.channel as {
      id: number;
    };
    const unregister = vi.spyOn(internals, "unregisterCallback");
    ui.stall();
    await clock.advance(WATCHDOG_MS);
    expect(unregister).toHaveBeenCalledWith(first.id);
  });

  test("drops the frames of a channel it replaced", async () => {
    const { ui, clock, client } = setup({ ride: rideView() });
    await flush();
    const old = ui.calls.find((c) => c.cmd === "ui_open")?.args.channel as {
      onmessage: (f: unknown) => void;
    };
    ui.stall();
    await clock.advance(WATCHDOG_MS);
    ui.resume();
    await flush();
    expect(ui.channel()).toBe(2);
    const before = client.snapshot;
    old.onmessage({ ...ui.frames[0], ride: null });
    expect(client.snapshot).toBe(before);
  });

  test("StrictMode's mount, unmount and mount open one channel", async () => {
    const ui = installUi({ ride: rideView() });
    const client = new UiClient(new FakeClock());
    const wrapper = ({ children }: { children: ReactNode }) => (
      <StrictMode>
        <UiProvider client={client}>{children}</UiProvider>
      </StrictMode>
    );
    const { result } = renderHook(() => useUi(), { wrapper });
    await flush();
    expect(ui.count("ui_open")).toBe(1);
    expect(result.current.ride?.id).toBe("ride-1");
  });
});

describe("refusals", () => {
  test("ui_open's storage (the keeper cannot read the pairings) is the channel's error", async () => {
    const { client } = setup({ keeperStorage: true });
    await flush();
    expect(client.snapshot.error).toEqual({
      code: "storage",
      message: "database is locked",
    });
    expect(client.snapshot.received).toBe(false);
  });

  test("ui_open's conflict beyond 16 channels is the channel's error, and the watchdog opens again", async () => {
    const { ui, clock, client } = setup({ otherWindows: MAX_CHANNELS });
    await flush();
    expect(client.snapshot.error?.code).toBe("conflict");
    vi.spyOn(console, "warn").mockImplementation(() => {});
    ui.truth.otherWindows = MAX_CHANNELS - 1;
    await clock.advance(WATCHDOG_MS);
    expect(ui.count("ui_open")).toBe(2);
    expect(client.snapshot.error).toBeNull();
    expect(client.snapshot.received).toBe(true);
  });

  test("ui_ack's not_found on a channel the core has closed is never shown", async () => {
    const { ui, client } = setup({ ride: rideView() });
    // The page loads elsewhere (or a send failed): the core closed the channel before the ack.
    const release = ui.hold("ui_ack");
    await flush();
    ui.pageLoad();
    release();
    await flush();
    expect(client.snapshot.error).toBeNull();
  });
});

describe("the watchdog (ADR-0001 §4.3.4)", () => {
  test("a heartbeat each second keeps the channel; 2 s of silence marks it stale and reopens", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const { ui, clock, client } = setup({ ride: rideView() });
    await flush();
    for (let s = 0; s < 5; s++) {
      await clock.advance(1000);
      ui.beat();
      await flush();
    }
    expect(ui.count("ui_open")).toBe(1);
    expect(client.snapshot.stale).toBe(false);
    // The core stops sending.
    ui.stall();
    await clock.advance(WATCHDOG_MS - 1);
    expect(client.snapshot.stale).toBe(false);
    await clock.advance(1);
    expect(client.snapshot.stale).toBe(true);
    expect(client.snapshot.reopens).toBe(1);
    expect(ui.count("ui_open")).toBe(2);
    expect(warn).toHaveBeenCalledWith(
      "ui channel: no frame for 2000 ms, reopening (1 so far)",
    );
    // It recovers with the next frame, on the new channel.
    ui.resume();
    await flush();
    expect(client.snapshot.stale).toBe(false);
    expect(ui.frames.at(-1)?.channel).toBe(2);
  });

  test("a dropped frame mid-ride: within 3 s the page holds the current state and the complete series", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const { ui, clock, client } = setup({
      ride: rideView({ status: status() }),
    });
    appendPower(ui, 40);
    await flush();
    // The next frame is lost (Tauri's fetch path): unacked, the channel would stall for good.
    ui.drop();
    appendPower(ui, 8, 10);
    ui.setStatus({ workoutS: 12 });
    await flush();
    const lostAt = clock.now;
    // The ride goes on; nothing reaches the page until the watchdog reopens.
    appendPower(ui, 4, 12);
    ui.setStatus({ workoutS: 13 });
    await clock.advance(WATCHDOG_MS);
    await flush();
    expect(clock.now - lostAt).toBeLessThanOrEqual(3000);
    expect(client.snapshot.stale).toBe(false);
    expect(client.snapshot.ride?.status.workoutS).toBe(13);
    expect(values(client.snapshot.series.power)).toEqual(coreValues(ui));
  });

  test("a page reload mid-ride backfills the series from index 0, contiguously", async () => {
    const first = setup({ ride: rideView() });
    appendPower(first.ui, 1000);
    await flush();
    // The reload: the page-load event closes the old page's channel; the new page has a new client.
    first.ui.pageLoad();
    first.release();
    const client = new UiClient(new FakeClock());
    client.retain();
    await flush();
    const opened = first.ui.frames.filter((f) => f.channel === 2);
    expect(opened.map((f) => f.series.power.from)).toEqual([0, 256, 512, 768]);
    expect(values(client.snapshot.series.power)).toEqual(coreValues(first.ui));
  });

  test("a forced end is visible after a reload: the new channel's ride says forced", async () => {
    const first = setup({ ride: rideView() });
    await flush();
    first.ui.setRide({ ended: "forced" });
    await flush();
    first.ui.pageLoad();
    first.release();
    const client = new UiClient(new FakeClock());
    const wrapper = ({ children }: { children: ReactNode }) => (
      <UiProvider client={client}>{children}</UiProvider>
    );
    const Ended = () => <output>{useUi().ride?.ended ?? "none"}</output>;
    const { container } = render(<Ended />, { wrapper });
    await flush();
    expect(container.textContent).toBe("forced");
  });
});
