// The snapshot channel on the real desktop app (ADR-0001 §4.3, §4.3.9; #598), with the simulated trainer: one ride,
// loaded and started through the UI, then, in order: the freeze, the 2 s watchdog when the channel stalls, a frame lost
// on Tauri's fetch path, a 32 KB frame's round trip, and a page reload mid-ride. Gated like every spec here: it runs
// only under `just e2e` (GUI session, #35/#261), never in CI or `pnpm test`.
//
// The page is the production frontend. The spec reads the page's own state through its read-only probe
// (`window.__crankUi()`, src/main.tsx) and watches the IPC callbacks Tauri delivers (`__TAURI_INTERNALS__.callbacks`,
// tauri 2.12.3 scripts/core.js): every frame the core sends reaches the page as a callback, a small one directly, one of
// 8 192 B or more as the answer of a `plugin:__TAURI_CHANNEL__|fetch` invoke (src/ipc/channel.rs `channel_on`). Not
// calling the page's channel callback for one frame is exactly a frame lost on that path: the page never sees it,
// never acks it, and the core, with an ack window of 1, sends nothing more on that channel.
import { browser, $, expect } from "@wdio/globals";

/** Runs in the page: the instrumentation, installed once per page load. */
function instrument() {
  if (window.__e2e) return;
  const callbacks = window.__TAURI_INTERNALS__.callbacks;
  const get = callbacks.get.bind(callbacks);
  const set = callbacks.set.bind(callbacks);
  const e2e = {
    /** Every UI frame the page's channel callbacks were handed, with when. */
    frames: [],
    /** Drop the next frame that is at least this many bytes of JSON (`null`: none). */
    dropAtLeast: null,
    dropped: [],
    /** Each callback registered (an invoke's answer, a channel) and run, for the round trip. */
    events: [],
  };
  const isFrame = (m) =>
    m && typeof m === "object" && "series" in m && "devices" in m;
  callbacks.set = (id, fn) => {
    e2e.events.push({ kind: "set", id, t: performance.now() });
    return set(id, fn);
  };
  callbacks.get = (id) => {
    const fn = get(id);
    if (!fn) return fn;
    return (data) => {
      const t = performance.now();
      const message =
        data && typeof data === "object" ? data.message : undefined;
      if (isFrame(message)) {
        const bytes = JSON.stringify(message).length;
        if (e2e.dropAtLeast !== null && bytes >= e2e.dropAtLeast) {
          e2e.dropAtLeast = null;
          e2e.dropped.push({
            t,
            bytes,
            channel: message.channel,
            seq: message.seq,
          });
          return undefined;
        }
        e2e.frames.push({ t, bytes, frame: message });
      }
      e2e.events.push({
        kind: "run",
        id,
        t,
        data: isFrame(data) ? "frame" : data,
      });
      return fn(data);
    };
  };
  window.__e2e = e2e;
}

const page = (fn, ...args) => browser.execute(fn, ...args);
const ui = () => page(() => window.__crankUi());
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Wait until `pred(ui snapshot)` holds, polling every 100 ms, at most `ms`; returns the snapshot and the time it took. */
async function until(pred, ms, what) {
  const start = Date.now();
  for (;;) {
    const s = await ui();
    if (pred(s)) return { s, took: Date.now() - start };
    if (Date.now() - start > ms)
      throw new Error(`timed out after ${ms} ms: ${what}`);
    await sleep(100);
  }
}

/** The page's series are one ride's, contiguous: power at 4 Hz, cadence and heart rate at 2 Hz, no hole and no repeat. */
function contiguous(series) {
  const step = { power: 0.25, cadence: 0.5, hr: 0.5 };
  for (const [name, points] of Object.entries(series))
    for (let i = 1; i < points.length; i++) {
      const d = points[i].t - points[i - 1].t;
      if (Math.abs(d - step[name]) > 1e-6)
        return `${name}: ${points[i - 1].t} -> ${points[i].t} at ${i}`;
    }
  return null;
}

/** Each series holds the frames' points exactly: `from` 0 replaces, any other `from` is the page's length. */
function replay(frames, channel) {
  const out = { power: [], cadence: [], hr: [] };
  for (const { frame } of frames.filter((f) => f.frame.channel === channel))
    for (const name of Object.keys(out)) {
      const p = frame.series[name];
      if (p.from === 0) out[name] = [];
      if (p.from !== out[name].length)
        return `${name}: from ${p.from} on ${out[name].length}`;
      out[name].push(...p.v);
    }
  return out;
}

describe("the snapshot channel", function () {
  this.timeout(300_000);

  before(async () => {
    // The simulator's devices, paired through the commands (the pairing UI is #471).
    await browser.executeAsync((done) => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      invoke("device_pair", { id: "trainer", role: "trainer" })
        .then(() => invoke("device_pair", { id: "strap", role: "heart_rate" }))
        .then(
          () => done(null),
          (e) => done(JSON.stringify(e)),
        );
    });
    await page(instrument);
    // Load and start a 45-minute ride through the UI.
    await $("nav[aria-label='Places']").$("button*=Workouts").click();
    await $("[role='option']*=Endurance 45").click();
    await $("button*=Ride this").click();
    await $("[data-testid='ride']").waitForDisplayed({ timeout: 30_000 });
    const start = $("button*=Start");
    await start.waitForEnabled({ timeout: 30_000 });
    await start.click();
    await until(
      (s) => s.ride?.status.state.kind === "riding",
      10_000,
      "riding",
    );
  });

  it("loses no state and no point across a 1 s webview freeze", async () => {
    await sleep(3000);
    const before = await ui();
    // Freeze the page: nothing runs, nothing is acked.
    await page(() => {
      const until = performance.now() + 1000;
      while (performance.now() < until);
    });
    await sleep(1500);
    const after = await ui();
    expect(after.stale).toBe(false);
    expect(after.series.power.length).toBeGreaterThan(
      before.series.power.length,
    );
    expect(contiguous(after.series)).toBeNull();
    // The page's state is the last frame's (the core's state at its send) and its series the frames' points.
    const { last, frames } = await page(() => ({
      last: window.__e2e.frames.at(-1).frame,
      frames: window.__e2e.frames,
    }));
    const replayed = replay(frames, last.channel);
    const now = await ui();
    expect(now.ride).toEqual(last.ride);
    expect(now.devices).toEqual(last.devices);
    expect(now.series.power.map((p) => p.v)).toEqual(replayed.power);
  });

  it("a stalled channel shows the stale state within 2 s and recovers on a new channel", async () => {
    const before = await ui();
    const channel = await page(() => window.__e2e.frames.at(-1).frame.channel);
    // The next frame never arrives: unacked, the core holds everything back.
    await page(() => {
      window.__e2e.dropAtLeast = 0;
    });
    await $("[data-testid='engine-unresponsive']").waitForDisplayed({
      timeout: 3500,
    });
    const stale = await ui();
    expect(stale.stale).toBe(true);
    expect(stale.reopens).toBe(before.reopens + 1);
    const { s, took } = await until((x) => !x.stale, 5000, "recovered");
    expect(took).toBeLessThan(5000);
    await $("[data-testid='engine-unresponsive']").waitForDisplayed({
      reverse: true,
      timeout: 3000,
    });
    const newer = await page(() => window.__e2e.frames.at(-1).frame.channel);
    expect(newer).toBeGreaterThan(channel);
    expect(s.ride.status.state.kind).toBe("riding");
  });

  it("a frame lost on the fetch path is recovered within 3 s, with the current state and a complete series", async () => {
    // Ride until a backfill frame is past Tauri's 8 192 B direct-eval limit and the frames are near 32 KB: 256 points
    // per series, which needs 128 s of cadence and heart rate at 2 Hz.
    await until(
      (s) => s.series.hr.length >= 300,
      200_000,
      "300 heart-rate points",
    );
    // Drop the first fetched frame of the next backfill; force a backfill by losing a small frame first.
    await page(() => {
      window.__e2e.dropAtLeast = 0;
    });
    await until((s) => s.stale, 3500, "stale");
    await page(() => {
      window.__e2e.dropAtLeast = 8192;
    });
    const lost = await browser.waitUntil(
      () =>
        page(
          () =>
            window.__e2e.dropped.at(-1)?.bytes >= 8192 &&
            window.__e2e.dropped.at(-1),
        ),
      { timeout: 5000, timeoutMsg: "no fetched frame was dropped" },
    );
    const lostAt = Date.now();
    const { s } = await until(
      (x) =>
        !x.stale &&
        x.series.power.length > 0 &&
        contiguous(x.series) === null &&
        x.series.power.at(-1).t >= x.ride.status.sessionS - 1,
      6000,
      "the series complete again",
    );
    const took = Date.now() - lostAt;
    console.log(
      `dropped a ${lost.bytes} B frame; recovered in ${took} ms (watchdog ${2000} ms + backfill)`,
    );
    expect(took).toBeLessThanOrEqual(3000);
    expect(s.ride.status.state.kind).toBe("riding");
  });

  it("a 32 KB frame's round trip is at most 100 ms (ADR-0001 §4.3.3)", async () => {
    // A backfill: lose one small frame, the watchdog reopens, the new channel sends the ride's series 256 at a time.
    const from = await page(() => window.__e2e.events.length);
    await page(() => {
      window.__e2e.dropAtLeast = 0;
    });
    await until((s) => s.stale, 3500, "stale");
    await until(
      (s) => !s.stale && s.series.hr.length >= 300,
      8000,
      "backfilled",
    );
    // Page-side round trip of each fetched frame: from the fetch invoke's registration (when the page learns of the
    // frame) to the answer of the `ui_ack` registered after the frame was applied.
    const trips = await page((from) => {
      const ev = window.__e2e.events.slice(from);
      const setAt = new Map(
        ev.filter((e) => e.kind === "set").map((e) => [e.id, e.t]),
      );
      const out = [];
      for (let i = 0; i < ev.length; i++) {
        const e = ev[i];
        if (e.kind !== "run" || e.data !== "frame" || !setAt.has(e.id))
          continue;
        // `e` is the fetch's answer, carrying the frame; the next null answer of an invoke set after it is the ack's.
        const ack = ev
          .slice(i + 1)
          .find(
            (x) =>
              x.kind === "run" &&
              x.data === null &&
              (setAt.get(x.id) ?? -1) >= e.t,
          );
        if (ack) out.push(ack.t - setAt.get(e.id));
      }
      return out;
    }, from);
    const sizes = await page(() =>
      window.__e2e.frames.filter((f) => f.bytes >= 8192).map((f) => f.bytes),
    );
    const big = Math.max(...sizes);
    const worst = Math.max(...trips);
    console.log(
      `round trips of ${trips.length} fetched frames (largest ${big} B): worst ${worst.toFixed(1)} ms, median ${trips.sort((a, b) => a - b)[trips.length >> 1].toFixed(1)} ms`,
    );
    expect(trips.length).toBeGreaterThan(0);
    expect(big).toBeGreaterThan(30_000);
    expect(worst).toBeLessThanOrEqual(100);
  });

  it("a page reload mid-ride backfills the series from index 0, contiguous, with the ride", async () => {
    const before = await ui();
    await browser.refresh();
    await page(instrument);
    const { s } = await until(
      (x) =>
        x.received &&
        x.ride !== null &&
        x.series.power.length >= before.series.power.length,
      15_000,
      "backfilled after the reload",
    );
    expect(s.ride.id).toBe(before.ride.id);
    expect(s.series.power[0].t).toBe(before.series.power[0].t);
    expect(contiguous(s.series)).toBeNull();
    // End the ride through the commands; the frame says how it ended.
    await browser.executeAsync((done) =>
      window.__TAURI_INTERNALS__.invoke("ride_end").then(
        () => done(null),
        (e) => done(JSON.stringify(e)),
      ),
    );
    await until((x) => x.ride?.ended === "finished", 5000, "ended");
  });
});
