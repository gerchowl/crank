// The UI's intent sources (ADR-0002 §5.1.2, #680): the view mapping, and the reporter on a fake clock with fake focus,
// visibility and input. Each test names the §5.1.8 acceptance test whose UI half it is: the core's half (the keeper's
// timers and phases) is tested in crates/crank-core/src/keeper; here, that the page sends what those tests start from.
import { describe, expect, test, vi } from "vitest";
import type { ApiError, LinkIntent, RideView } from "../bindings";
import type { Route } from "../router";
import { rideView } from "../stream/uiBackend";
import { status } from "../stream/testBackend";
import {
  IntentReporter,
  LIKELY_REFRESH_MS,
  Throttle,
  viewIntent,
  type IntentEnv,
} from "./intent";

const LIKELY: LinkIntent = { kind: "likely", ready: false };
const READY: LinkIntent = { kind: "likely", ready: true };
const LEFT: LinkIntent = { kind: "left" };
const BACKGROUND: LinkIntent = { kind: "background" };
const HIDDEN: LinkIntent = { kind: "release", reason: "hidden" };
const DEVICES: LinkIntent = { kind: "release", reason: "devices" };

describe("viewIntent", () => {
  const at = (view: Route["view"], sub?: string): Route =>
    sub === undefined ? { view } : { view, sub };
  const started = rideView({
    status: status({ state: { kind: "riding" } }),
  });
  const ended = rideView({
    status: status({ state: { kind: "finished" } }),
    ended: "finished",
  });

  test.each<[string, Route, RideView | null, boolean, LinkIntent | null]>([
    ["Today", at("today"), null, false, LIKELY],
    ["Workouts", at("workouts"), null, false, LIKELY],
    ["a selected workout", at("workouts", "w-ss"), null, false, LIKELY],
    [
      "the Ready sheet over a loaded ride",
      at("ride"),
      rideView(),
      false,
      LIKELY,
    ],
    ["the Ready sheet over a load in flight", at("ride"), null, true, READY],
    ["Settings → Devices", at("settings", "devices"), null, false, DEVICES],
    ["Settings → Appearance", at("settings"), null, false, LEFT],
    ["Settings → Athlete model", at("settings", "athlete"), null, false, LEFT],
    ["History", at("history"), null, false, LEFT],
    ["a ride in History", at("history", "ride-1"), null, false, LEFT],
    ["Onboarding", at("onboarding"), null, false, LEFT],
    ["the Resume sheet", at("resume"), null, false, LEFT],
    ["a started ride", at("ride"), started, false, LEFT],
    ["an ended ride", at("ride"), ended, false, LEFT],
    ["the ride screen with no ride", at("ride"), null, false, LEFT],
  ])("%s", (_, route, ride, loading, intent) => {
    expect(viewIntent(route, { received: true, ride, loading })).toEqual(
      intent,
    );
  });

  test("a warm view stays Likely while a load is in flight elsewhere", () => {
    // `loading` is the core's; only the Ready sheet holds without input.
    expect(
      viewIntent(at("workouts"), { received: true, ride: null, loading: true }),
    ).toEqual(LIKELY);
  });

  test("the ride screen before the first frame says nothing; the places need no frame", () => {
    const none = { received: false, ride: null, loading: false };
    expect(viewIntent(at("ride"), none)).toBeNull();
    expect(viewIntent(at("today"), none)).toEqual(LIKELY);
    expect(viewIntent(at("history"), none)).toEqual(LEFT);
  });
});

describe("Throttle", () => {
  test("lets one through per window, counted from the last mark", () => {
    let t = 0;
    const th = new Throttle(1000, () => t);
    expect(th.pass()).toBe(true);
    t = 999;
    expect(th.pass()).toBe(false);
    t = 1000;
    expect(th.pass()).toBe(true);
    t = 1500;
    th.mark();
    t = 2499;
    expect(th.pass()).toBe(false);
    t = 2500;
    expect(th.pass()).toBe(true);
  });
});

/** A fake window: a clock, focus, visibility and input the test drives, and `links_intent` answered on demand. */
function fakeEnv(opts: { hidden?: boolean; hold?: boolean } = {}) {
  let t = 0;
  let focus: ((f: boolean) => void) | null = null;
  let visibility: ((h: boolean) => void) | null = null;
  let input: (() => void) | null = null;
  const sent: LinkIntent[] = [];
  const pending: ((r: ApiError | null) => void)[] = [];
  const env: IntentEnv = {
    send(intent) {
      sent.push(intent);
      if (!opts.hold) return Promise.resolve(null);
      return new Promise((r) => pending.push(r));
    },
    now: () => t,
    watchFocus(cb) {
      focus = cb;
      return () => (focus = null);
    },
    watchVisibility(cb) {
      visibility = cb;
      cb(opts.hidden ?? false);
      return () => (visibility = null);
    },
    watchInput(cb) {
      input = cb;
      return () => (input = null);
    },
  };
  return {
    env,
    sent,
    advance(ms: number) {
      t += ms;
    },
    focus: (f: boolean) => focus?.(f),
    hide: (h: boolean) => visibility?.(h),
    input: () => input?.(),
    /** Answer the oldest call in flight. */
    async answer(r: ApiError | null = null) {
      pending.shift()?.(r);
      await Promise.resolve();
      await Promise.resolve();
    },
    get subscribed() {
      return focus !== null && visibility !== null && input !== null;
    },
  };
}

const flush = async () => {
  for (let i = 0; i < 3; i++) await Promise.resolve();
};

describe("IntentReporter", () => {
  test("sends the view's intent once per change, and nothing while it is not known", async () => {
    const w = fakeEnv();
    const r = new IntentReporter(w.env);
    r.start();
    r.setView(null);
    w.focus(false);
    w.focus(true);
    await flush();
    expect(w.sent).toEqual([]);
    r.setView(LIKELY);
    await flush();
    r.setView(LIKELY);
    r.setView({ kind: "likely", ready: false });
    await flush();
    r.setView(LEFT);
    await flush();
    r.setView(DEVICES);
    await flush();
    expect(w.sent).toEqual([LIKELY, LEFT, DEVICES]);
  });

  // §5.1.8 "Idle cool-down disconnects": the core starts `T_cool` on `Left`, and "a `Likely` in between restarts
  // the timer": coming back to a warm view sends a fresh `Likely`.
  test("leaving a warm view sends Left, coming back sends a fresh Likely", async () => {
    const w = fakeEnv();
    const r = new IntentReporter(w.env);
    r.start();
    r.setView(LIKELY);
    await flush();
    r.setView(LEFT);
    await flush();
    r.setView(LIKELY);
    await flush();
    expect(w.sent).toEqual([LIKELY, LEFT, LIKELY]);
  });

  // §5.1.8 "Background grace": `Background` starts the 60 s grace, and "a `Likely` at 30 s cancels the release".
  test("losing focus sends Background, regaining it sends Likely again", async () => {
    const w = fakeEnv();
    const r = new IntentReporter(w.env);
    r.start();
    r.setView(LIKELY);
    await flush();
    w.focus(false);
    await flush();
    w.advance(30_000);
    w.focus(true);
    await flush();
    expect(w.sent).toEqual([LIKELY, BACKGROUND, LIKELY]);
    // Back on a view that is not warm, focus brings back that view's intent.
    r.setView(LEFT);
    await flush();
    w.focus(false);
    await flush();
    w.focus(true);
    await flush();
    expect(w.sent.slice(3)).toEqual([LEFT, BACKGROUND, LEFT]);
  });

  // §5.1.8 "Every `Release` reason disconnects at once": the page's two, `Hidden` and `Devices`.
  test("hiding the window sends Release(Hidden), whatever the view or focus; showing it sends the view's intent", async () => {
    const w = fakeEnv();
    const r = new IntentReporter(w.env);
    r.start();
    r.setView(LIKELY);
    await flush();
    w.hide(true);
    await flush();
    // hidden outranks unfocused and any view
    w.focus(false);
    r.setView(LEFT);
    r.setView(LIKELY);
    await flush();
    expect(w.sent).toEqual([LIKELY, HIDDEN]);
    w.hide(false);
    await flush();
    expect(w.sent.at(-1)).toEqual(BACKGROUND);
    w.focus(true);
    await flush();
    expect(w.sent).toEqual([LIKELY, HIDDEN, BACKGROUND, LIKELY]);
  });

  test("a window hidden when the reporter starts sends Release(Hidden) first", async () => {
    const w = fakeEnv({ hidden: true });
    const r = new IntentReporter(w.env);
    r.start();
    r.setView(LIKELY);
    await flush();
    expect(w.sent).toEqual([HIDDEN]);
  });

  // §5.1.8 "An abandoned load releases … Input at 9 min restarts the timer" and the warm views' `T_cool`: rider
  // input refreshes the standing `Likely`, at most once per 30 s.
  test("rider input on a warm view sends Likely again at most once per 30 s", async () => {
    const w = fakeEnv();
    const r = new IntentReporter(w.env);
    r.start();
    r.setView(LIKELY);
    await flush();
    w.input();
    w.advance(LIKELY_REFRESH_MS - 1);
    w.input();
    await flush();
    expect(w.sent).toEqual([LIKELY]);
    w.advance(1);
    w.input();
    w.input();
    await flush();
    expect(w.sent).toEqual([LIKELY, LIKELY]);
    // the window counts from the last Likely sent, also one sent for a change
    w.advance(LIKELY_REFRESH_MS - 10);
    r.setView(LEFT);
    r.setView(LIKELY);
    await flush();
    w.advance(20);
    w.input();
    await flush();
    expect(w.sent).toEqual([LIKELY, LIKELY, LEFT, LIKELY]);
    w.advance(LIKELY_REFRESH_MS);
    w.input();
    await flush();
    expect(w.sent).toEqual([LIKELY, LIKELY, LEFT, LIKELY, LIKELY]);
  });

  test("input sends nothing on a view that is not warm, nor while hidden or in the background", async () => {
    const w = fakeEnv();
    const r = new IntentReporter(w.env);
    r.start();
    for (const view of [LEFT, DEVICES]) {
      r.setView(view);
      w.advance(LIKELY_REFRESH_MS);
      w.input();
      await flush();
    }
    w.focus(false);
    r.setView(LIKELY);
    w.advance(LIKELY_REFRESH_MS);
    w.input();
    await flush();
    w.hide(true);
    w.advance(LIKELY_REFRESH_MS);
    w.input();
    await flush();
    expect(w.sent).toEqual([LEFT, DEVICES, BACKGROUND, HIDDEN]);
  });

  // §5.1.8 "The Ready sheet holds": over a load in flight or failed it is `Likely { ready: true }`, which holds
  // without input; the page sends it once and nothing more for 30 min of no input.
  test("the Ready sheet's Likely { ready: true } is sent once and needs no input to hold", async () => {
    const w = fakeEnv();
    const r = new IntentReporter(w.env);
    r.start();
    r.setView(READY);
    for (let i = 0; i < 30; i++) {
      w.advance(60_000);
      r.setView(READY);
      await flush();
    }
    expect(w.sent).toEqual([READY]);
    // input refreshes it as it stands, `ready` included
    w.input();
    await flush();
    expect(w.sent).toEqual([READY, READY]);
  });

  test("one call at a time: a change waits for the answer, and only the latest waits", async () => {
    const w = fakeEnv({ hold: true });
    const r = new IntentReporter(w.env);
    r.start();
    r.setView(LIKELY);
    r.setView(LEFT);
    r.setView(DEVICES);
    await flush();
    expect(w.sent).toEqual([LIKELY]);
    await w.answer();
    expect(w.sent).toEqual([LIKELY, DEVICES]);
    await w.answer();
    expect(w.sent).toEqual([LIKELY, DEVICES]);
    // a release answered late is never overtaken by the Likely after it
    r.setView(LIKELY);
    await flush();
    expect(w.sent).toEqual([LIKELY, DEVICES, LIKELY]);
  });

  test("a refusal is logged and not retried", async () => {
    const w = fakeEnv({ hold: true });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const r = new IntentReporter(w.env);
    r.start();
    r.setView(LIKELY);
    await flush();
    await w.answer({
      code: "conflict",
      message: "the application is quitting",
    });
    await flush();
    expect(w.sent).toEqual([LIKELY]);
    expect(warn).toHaveBeenCalledWith(
      'links_intent {"kind":"likely","ready":false} refused: conflict: the application is quitting',
    );
    warn.mockRestore();
  });

  test("stop unsubscribes from focus, visibility and input", () => {
    const w = fakeEnv();
    const stop = new IntentReporter(w.env).start();
    expect(w.subscribed).toBe(true);
    stop();
    expect(w.subscribed).toBe(false);
  });
});
