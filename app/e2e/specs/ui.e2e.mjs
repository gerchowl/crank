// The snapshot channel on the real desktop app (ADR-0001 §4.3, §4.3.9; #598), with the simulated trainer: one ride,
// loaded and started through the UI, then, in order: the freeze, the 2 s watchdog when the channel stalls, a frame lost
// on Tauri's fetch path, the round trip of the largest frames, and a page reload mid-ride. Gated like every spec here:
// it runs only under `just e2e` (GUI session, #35/#261), never in CI or `pnpm test`. It rides about three minutes.
//
// The page is the production frontend. The spec reads the page's own state through its read-only probe
// (`window.__crankUi()`, src/main.tsx, ADR-0001 §4.3.2) and watches the IPC callbacks Tauri delivers
// (`__TAURI_INTERNALS__.callbacks`, a `Map` that tauri 2.12.3 scripts/core.js `runCallback` reads with `get`): every
// frame the core sends reaches the page as a callback, a small one directly, one of 8 192 B or more as the answer of a
// `plugin:__TAURI_CHANNEL__|fetch` invoke (src/ipc/channel.rs `channel_on`). Not calling the page's channel callback for
// one frame is exactly a frame lost on that path: the page never sees it, never acks it, and the core, with an ack
// window of 1, sends nothing more on that channel.
import { browser, $, expect } from "@wdio/globals";

/** Runs in the page: the instrumentation, installed once per page load. */
function instrument() {
  if (window.__e2e) return;
  const callbacks = window.__TAURI_INTERNALS__.callbacks;
  const get = callbacks.get.bind(callbacks);
  const set = callbacks.set.bind(callbacks);
  const e2e = {
    /** Every UI frame the page's channel callbacks were handed, with when (`performance.now()`) and its size. */
    frames: [],
    /** Frames to lose, in order: each entry loses the next frame of at least that many bytes of JSON. */
    drops: [],
    /** Until this `performance.now()`, every frame is lost: the channel stalls, whatever the watchdog reopens. */
    stallUntil: 0,
    dropped: [],
    /** Each callback registered (an invoke's answer, a channel) and run, for the round trip. */
    events: [],
  };
  const isFrame = (m) =>
    m !== null && typeof m === "object" && "series" in m && "devices" in m;
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
        data !== null && typeof data === "object" ? data.message : undefined;
      if (isFrame(message)) {
        const bytes = JSON.stringify(message).length;
        if (
          t < e2e.stallUntil ||
          (e2e.drops.length > 0 && bytes >= e2e.drops[0])
        ) {
          if (t >= e2e.stallUntil) e2e.drops.shift();
          e2e.dropped.push({ t, bytes, channel: message.channel });
          return undefined;
        }
        e2e.frames.push({ t, bytes, frame: message });
      }
      e2e.events.push({
        kind: "run",
        id,
        t,
        data: isFrame(data) ? "frame" : data === null ? null : "other",
      });
      return fn(data);
    };
  };
  window.__e2e = e2e;
}

const page = (fn, ...args) => browser.execute(fn, ...args);
/** The page's state, with the page's clock at the read; `received: false` while a reloaded page has not booted. */
const ui = () =>
  page(() =>
    typeof window.__crankUi === "function"
      ? { ...window.__crankUi(), now: performance.now() }
      : { received: false, ride: null, stale: false, now: performance.now() },
  );
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Wait until `pred(ui snapshot)` holds, polling every 100 ms, at most `ms`. */
async function until(pred, ms, what) {
  const start = Date.now();
  for (;;) {
    const s = await ui();
    if (pred(s)) return s;
    if (Date.now() - start > ms)
      throw new Error(`timed out after ${ms} ms: ${what}`);
    await sleep(100);
  }
}

/**
 * The page's series are one ride's, contiguous: power at 4 Hz and cadence at 2 Hz with no hole and no repeat; heart
 * rate at 2 Hz on the same grid, strictly increasing, with holes allowed: the simulated strap drops out for 2 to 6
 * readings now and then (crates/crank-devices/src/sim/core.rs), and a reading it never sent is no point.
 */
function contiguous(series) {
  const step = { power: 0.25, cadence: 0.5, hr: 0.5 };
  for (const [name, points] of Object.entries(series))
    for (let i = 1; i < points.length; i++) {
      const d = points[i].t - points[i - 1].t;
      const n = d / step[name];
      const ok =
        name === "hr"
          ? n >= 1 - 1e-6 && Math.abs(n - Math.round(n)) < 1e-6
          : Math.abs(d - step[name]) < 1e-6;
      if (!ok) return `${name}: ${points[i - 1].t} -> ${points[i].t} at ${i}`;
    }
  return null;
}

/** A series as the frames of `channel` build it by the client rule; a `from` that is not the length is an error. */
function replay(frames, channel, name) {
  let out = [];
  for (const { frame } of frames.filter((f) => f.frame.channel === channel)) {
    const p = frame.series[name];
    if (p.from === 0) out = [];
    if (p.from !== out.length)
      throw new Error(`${name}: from ${p.from} on ${out.length}`);
    out.push(...p.v);
  }
  return out;
}

/**
 * Lose the next frame of any size: the channel stalls, the watchdog reopens it 2 s after the last frame. The new
 * channel's first frame follows within milliseconds, so the stale state of one lost frame lasts too short to see;
 * a test that needs it stalls the channel instead (`stallUntil`).
 */
const loseOne = () =>
  page(() => {
    window.__e2e.drops.push(0);
  });

describe("the snapshot channel", function () {
  this.timeout(360_000);

  before(async () => {
    // The simulator's devices (sim backend: one fixed id per role), paired through the commands (the pairing UI is #471).
    const paired = await browser.executeAsync((done) => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      invoke("device_pair", { id: "trainer", role: "trainer" })
        .then(() => invoke("device_pair", { id: "strap", role: "heart_rate" }))
        .then(
          () => done(null),
          (e) => done(JSON.stringify(e)),
        );
    });
    expect(paired).toBeNull();
    await page(instrument);
    // Load and start a 45-minute ride through the UI.
    await $("nav[aria-label='Places']").$("button*=Workouts").click();
    await $(
      "//*[@role='option'][contains(normalize-space(.), 'Endurance 45')]",
    ).click();
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
      const end = performance.now() + 1000;
      while (performance.now() < end);
    });
    await sleep(1500);
    // In one read: the page's state, the last frame it applied and every frame of its channel.
    const r = await page(() => {
      const frames = window.__e2e.frames;
      return { now: window.__crankUi(), last: frames.at(-1).frame, frames };
    });
    expect(r.now.stale).toBe(false);
    expect(r.now.series.power.length).toBeGreaterThan(
      before.series.power.length,
    );
    expect(contiguous(r.now.series)).toBeNull();
    // The state is the last frame's (the core's at its send), the series exactly the frames' points.
    expect(r.now.ride).toEqual(r.last.ride);
    expect(r.now.devices).toEqual(r.last.devices);
    for (const name of ["power", "cadence", "hr"])
      expect(r.now.series[name].map((p) => p.v)).toEqual(
        replay(r.frames, r.last.channel, name),
      );
  });

  it("a stalled channel shows the stale state within 2 s and recovers on a new channel", async () => {
    const before = await ui();
    const channel = await page(() => window.__e2e.frames.at(-1).frame.channel);
    // For 3 s no frame arrives: unacked, the core holds everything back, and the frames of the channel the watchdog
    // reopens are lost too.
    const stalledAt = await page(() => {
      const t = performance.now();
      window.__e2e.stallUntil = t + 3000;
      return t;
    });
    await $("[data-testid='engine-unresponsive']").waitForDisplayed({
      timeout: 3500,
    });
    const stale = await ui();
    expect(stale.stale).toBe(true);
    expect(stale.reopens).toBeGreaterThanOrEqual(before.reopens + 1);
    console.log(
      `ui.e2e: stalled channel shown as stale ${Math.round(stale.now - stalledAt)} ms after the stall began`,
    );
    // The stall ends at 3 s; the next reopen (at most 2 s later) recovers.
    const s = await until((x) => !x.stale, 5000, "recovered");
    await $("[data-testid='engine-unresponsive']").waitForDisplayed({
      reverse: true,
      timeout: 3000,
    });
    const newer = await page(() => window.__e2e.frames.at(-1).frame.channel);
    expect(newer).toBeGreaterThan(channel);
    expect(s.ride.status.state.kind).toBe("riding");
  });

  it("a frame lost on the fetch path is recovered within 3 s, with the current state and a complete series", async () => {
    // Ride until a backfill's frames are past Tauri's 8 192 B direct-eval limit: 256 points per series, which needs
    // 128 s of cadence and heart rate at 2 Hz.
    await until(
      (s) => s.series.hr.length >= 300,
      200_000,
      "300 heart-rate points",
    );
    // Lose one frame (the watchdog reopens), then the first fetched frame of the backfill that follows.
    await page(() => {
      window.__e2e.drops.push(0, 8192);
    });
    await browser.waitUntil(() => page(() => window.__e2e.drops.length === 0), {
      timeout: 6000,
      timeoutMsg: "no fetched frame was lost",
    });
    const lost = await page(() => window.__e2e.dropped.at(-1));
    expect(lost.bytes).toBeGreaterThanOrEqual(8192);
    const s = await until(
      (x) =>
        !x.stale &&
        x.series.power.length > 0 &&
        contiguous(x.series) === null &&
        x.series.power.at(-1).t >= x.ride.status.sessionS - 1,
      8000,
      "the series complete again",
    );
    const took = s.now - lost.t;
    console.log(
      `ui.e2e: lost a ${lost.bytes} B frame; current and complete again after ${Math.round(took)} ms`,
    );
    expect(took).toBeLessThanOrEqual(3000);
    expect(s.ride.status.state.kind).toBe("riding");
  });

  it("the largest frames' round trip is at most 100 ms (ADR-0001 §4.3.3)", async () => {
    // A backfill: lose one frame, the watchdog reopens, the new channel sends the ride's series 256 points at a time.
    const from = await page(() => window.__e2e.events.length);
    const sizesFrom = await page(() => window.__e2e.frames.length);
    const reopens = (await ui()).reopens;
    await loseOne();
    await until((s) => s.reopens > reopens, 3500, "reopened");
    await until(
      (s) => !s.stale && s.series.hr.length >= 300,
      8000,
      "backfilled",
    );
    // Page-side round trip of each fetched frame: from the fetch invoke's registration (the page learns of the frame)
    // to the answer of the `ui_ack` registered after the frame was applied.
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
    const sizes = await page(
      (n) => window.__e2e.frames.slice(n).map((f) => f.bytes),
      sizesFrom,
    );
    const largest = Math.max(...sizes);
    const sorted = [...trips].sort((a, b) => a - b);
    const worst = sorted.at(-1);
    // The frame bound is 32 KB (§4.3.3); a sim ride of a few minutes has short session times, so its fullest frames
    // (256 points per series) are smaller: the line names the size measured.
    console.log(
      `ui.e2e: round trip of ${trips.length} fetched frames (largest frame ${largest} B): worst ${worst.toFixed(1)} ms, median ${sorted[sorted.length >> 1].toFixed(1)} ms`,
    );
    expect(trips.length).toBeGreaterThan(0);
    expect(largest).toBeGreaterThanOrEqual(8192);
    expect(worst).toBeLessThanOrEqual(100);
  });

  it("a page reload mid-ride backfills the series from index 0, contiguous, with the ride", async () => {
    const before = await ui();
    await browser.refresh();
    const s = await until(
      (x) =>
        x.received &&
        x.ride !== null &&
        x.series.power.length >= before.series.power.length,
      15_000,
      "backfilled after the reload",
    );
    await page(instrument);
    expect(s.ride.id).toBe(before.ride.id);
    expect(s.series.power[0].t).toBe(before.series.power[0].t);
    expect(contiguous(s.series)).toBeNull();
    // The backfill is the series the page held before the reload, point for point.
    for (const name of ["power", "cadence", "hr"])
      expect(s.series[name].slice(0, before.series[name].length)).toEqual(
        before.series[name],
      );
    // End the ride through the commands; the frame says how it ended.
    const ended = await browser.executeAsync((done) =>
      window.__TAURI_INTERNALS__.invoke("ride_end").then(
        () => done(null),
        (e) => done(JSON.stringify(e)),
      ),
    );
    expect(ended).toBeNull();
    await until((x) => x.ride?.ended === "finished", 5000, "ended");
  });
});
