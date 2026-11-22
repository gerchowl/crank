// The UI's intent sources for the link keeper (ADR-0002 §5.1.2, #680, part b of #471): what the window shows, whether it
// is visible and frontmost, and the rider's input become `links_intent` calls. The core owns every timer and decision
// (`T_cool`, the background grace, `T_abandon`); the page only says what is true now. Framework-free; `LinkIntents.tsx`
// binds it to the route and the snapshot channel.
//
// What is sent, the first rule that holds:
// - the window is hidden (`visibilitychange`: minimised, the app hidden): `Release(Hidden)`;
// - the window is not focused (Tauri's `onFocusChanged`: crank is not the frontmost app): `Background`;
// - otherwise the view's intent (`viewIntent`): `Likely` on a warm view (Today, Workouts, the Ready sheet), with
//   `ready` on the Ready sheet while no ride is loaded; `Release(Devices)` on Settings → Devices; `Left` anywhere else.
// Each is sent once when it becomes true (a change of view, focus or visibility), so coming back to a warm view or to
// the window is a fresh `Likely`. While it is `Likely`, rider input (a pointer, a key, the wheel) sends it again at most
// once per `LIKELY_REFRESH_MS`, counted from the last `Likely` sent: the core reads that refresh as rider input
// (`T_cool`, and `T_abandon` on the Ready sheet over a loaded ride). The refresh is leading-edge only: input inside the
// window is dropped, never deferred, so nothing is sent after the rider stopped.
//
// One call is in flight at a time and the core applies intents in the order sent: a call made while one is answered
// waits, and only the latest waits (each one is the whole truth of now, so an older queued one says nothing more). A
// release is answered once the links are let go (up to 2 s), so a quick Today → Devices never lets a `Likely` overtake
// the `Release(Devices)`. A refusal (`conflict` after the quit or for a superseded release, `storage` when the keeper
// cannot start) is logged and not retried: the intent's effect, or its absence, reaches the page as the snapshot
// channel's next frame (#597), and the next change sends again.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "../api";
import { commands } from "../bindings";
import type { ApiError, LinkIntent, RideView } from "../bindings";
import type { Route } from "../router";

/** At most one `Likely` per this long from rider input (ADR-0002 §5.1.2: "at most once per 30 s"). */
export const LIKELY_REFRESH_MS = 30_000;

const LEFT: LinkIntent = { kind: "left" };
const BACKGROUND: LinkIntent = { kind: "background" };
const HIDDEN: LinkIntent = { kind: "release", reason: "hidden" };
const DEVICES: LinkIntent = { kind: "release", reason: "devices" };
const likely = (ready: boolean): LinkIntent => ({ kind: "likely", ready });

/** What the ride screen reads of the snapshot channel's last frame (`UiSnapshot`). */
export interface RideFrame {
  /** A frame has arrived: until then the ride screen does not know what it shows. */
  received: boolean;
  ride: RideView | null;
  loading: boolean;
}

/**
 * What the view says of the rider's intent: the route, and on the ride screen the snapshot channel's ride; `null` on
 * the ride screen before the first frame, which does not know yet whether it is the Ready sheet (nothing is sent).
 */
export function viewIntent(
  route: Route,
  { received, ride, loading }: RideFrame,
): LinkIntent | null {
  switch (route.view) {
    case "today":
    case "workouts":
      return likely(false);
    case "settings":
      return route.sub === "devices" ? DEVICES : LEFT;
    case "ride":
      // The ride screen is the Ready sheet before Start (Ride.tsx): over a load that has not installed its ride yet it
      // holds without input (`ready`); over a loaded, unstarted ride its input is what `T_abandon` counts. A started or
      // ended ride, and the screen with no ride at all, are another view (`Left`, which the keeper ignores while a ride
      // holds the links, §5.1.2). §5.1.2 also holds the sheet over a failed load with `ready`, but the sheet draws no
      // failed load today: a refused `ride_load` is shown on Workouts or by the load notice (rideLoad.ts), and the ride
      // screen then has no ride. Where a refusal shows on the ride screen is #366; a sheet that draws it is `ready` too.
      if (!received) return null;
      if (ride === null) return loading ? likely(true) : LEFT;
      return ride.ended === null && ride.status.state.kind === "idle"
        ? likely(false)
        : LEFT;
    case "history":
    case "onboarding":
    case "resume":
      return LEFT;
  }
}

/** Lets one pass through per `ms` on an injected clock, counted from the last one marked; nothing is deferred. */
export class Throttle {
  private last = Number.NEGATIVE_INFINITY;
  constructor(
    private readonly ms: number,
    private readonly now: () => number,
  ) {}
  /** True, and marked, when `ms` have passed since the last mark. */
  pass(): boolean {
    if (this.now() - this.last < this.ms) return false;
    this.mark();
    return true;
  }
  /** Start the window now. */
  mark(): void {
    this.last = this.now();
  }
}

/** Where the reporter's inputs come from and where its intents go: the window's, or a test's. */
export interface IntentEnv {
  /** `links_intent`; resolves with its refusal or `null`, never rejects. */
  send(intent: LinkIntent): Promise<ApiError | null>;
  /** A monotonic clock in ms. */
  now(): number;
  /** Calls back on every focus change of the window; returns the unsubscribe. */
  watchFocus(cb: (focused: boolean) => void): () => void;
  /** Calls back with `hidden` now and on every visibility change; returns the unsubscribe. */
  watchVisibility(cb: (hidden: boolean) => void): () => void;
  /** Calls back on rider input; returns the unsubscribe. */
  watchInput(cb: () => void): () => void;
}

/** The window's own inputs and the core's command. */
export const WINDOW_ENV: IntentEnv = {
  async send(intent) {
    const r = await api(commands.linksIntent(intent));
    return r.status === "error" ? r.error : null;
  },
  now: () => performance.now(),
  watchFocus(cb) {
    // As displayMm.ts `watchMonitorChange`: outside Tauri (a plain browser, the tests) nothing is subscribed.
    let stopped = false;
    let off: (() => void) | null = null;
    try {
      getCurrentWindow()
        .onFocusChanged(({ payload }) => cb(payload))
        .then((unlisten) => (stopped ? unlisten() : (off = unlisten)))
        .catch(() => undefined);
    } catch {
      // not running inside Tauri
    }
    return () => {
      stopped = true;
      off?.();
    };
  },
  watchVisibility(cb) {
    const read = () => cb(document.visibilityState === "hidden");
    read();
    document.addEventListener("visibilitychange", read);
    return () => document.removeEventListener("visibilitychange", read);
  },
  watchInput(cb) {
    const kinds = ["pointerdown", "keydown", "wheel"] as const;
    const opts = { capture: true, passive: true };
    for (const k of kinds) window.addEventListener(k, cb, opts);
    return () => {
      for (const k of kinds) window.removeEventListener(k, cb, opts);
    };
  },
};

/** The window's intent reporter: feed it the view's intent; it watches focus, visibility and input itself. */
export class IntentReporter {
  private view: LinkIntent | null = null;
  // The window is assumed frontmost until Tauri says otherwise: it is when the app hands off to its first view.
  private focused = true;
  private hidden = false;
  /** The intent last sent for a change (JSON), so an unchanged one is not sent again. */
  private last: string | null = null;
  private readonly refresh: Throttle;
  private inFlight = false;
  private queued: LinkIntent | null = null;

  constructor(private readonly env: IntentEnv = WINDOW_ENV) {
    this.refresh = new Throttle(LIKELY_REFRESH_MS, env.now);
  }

  /** Watch focus, visibility and input until the returned function runs. */
  start(): () => void {
    const offs = [
      this.env.watchFocus((f) => {
        this.focused = f;
        this.update();
      }),
      this.env.watchVisibility((h) => {
        this.hidden = h;
        this.update();
      }),
      this.env.watchInput(() => this.input()),
    ];
    return () => {
      for (const off of offs) off();
    };
  }

  /** The view's intent (`viewIntent`), whenever it may have changed; `null` while it is not known. */
  setView(intent: LinkIntent | null): void {
    this.view = intent;
    this.update();
  }

  /** What is true now; `null` while the view's intent is not known. */
  private current(): LinkIntent | null {
    if (this.view === null) return null;
    if (this.hidden) return HIDDEN;
    if (!this.focused) return BACKGROUND;
    return this.view;
  }

  private update(): void {
    const now = this.current();
    if (now === null) return;
    const key = JSON.stringify(now);
    if (key === this.last) return;
    this.last = key;
    if (now.kind === "likely") this.refresh.mark();
    this.send(now);
  }

  /** Rider input: the standing `Likely` again, at most once per `LIKELY_REFRESH_MS`. */
  private input(): void {
    const now = this.current();
    if (now?.kind !== "likely" || !this.refresh.pass()) return;
    this.send(now);
  }

  private send(intent: LinkIntent): void {
    if (this.inFlight) {
      this.queued = intent;
      return;
    }
    this.inFlight = true;
    void this.env.send(intent).then((refused) => {
      if (refused !== null)
        console.warn(
          `links_intent ${JSON.stringify(intent)} refused: ${refused.code}: ${refused.message}`,
        );
      this.inFlight = false;
      const next = this.queued;
      this.queued = null;
      if (next !== null) this.send(next);
    });
  }
}
