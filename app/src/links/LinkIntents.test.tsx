// @vitest-environment jsdom
// The intent sources in the real app shell (ADR-0002 §5.1.2, #680): the route and the snapshot channel's ride decide
// the intent, the document's visibility releases, and rider input refreshes a warm view, each reaching the core's
// `links_intent` through the mock backends (stream/uiBackend.ts, ride/rideBackend.ts), which apply it as the keeper
// does. Focus, which needs Tauri's window events, and the 30 s clock are tested on the reporter itself (intent.test.ts).
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { App } from "../App";
import type { LinkIntent, PairedDevice } from "../bindings";
import { markSeen } from "../build";
import { installRide } from "../ride/rideBackend";
import { navigate } from "../router";
import {
  devicesOf,
  installUi,
  rideView,
  type UiBackend,
} from "../stream/uiBackend";
import { installDevices } from "../views/devicesBackend";
import { install as installToday } from "../views/todayBackend";

const BUILD = "0.2.0-rc.1 · 7516045";
const readBuild = () => Promise.resolve(BUILD);
const LIKELY: LinkIntent = { kind: "likely", ready: false };
const LEFT: LinkIntent = { kind: "left" };

const neo: PairedDevice = {
  role: "trainer",
  id: "neo-1",
  name: "Tacx Neo 55624",
  serial: "55624",
  model: "Neo",
};

type Invoke = (cmd: string, args?: unknown) => Promise<unknown>;
const invoke = () =>
  (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } })
    .__TAURI_INTERNALS__.invoke;

/** The intents the core received, in order. */
const intents = (calls: { cmd: string; args: Record<string, unknown> }[]) =>
  calls.filter((c) => c.cmd === "links_intent").map((c) => c.args.intent);

let visibility: DocumentVisibilityState = "visible";
const setVisibility = (v: DocumentVisibilityState) => {
  visibility = v;
  act(() => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
};

const ready = async () => {
  const main = await screen.findByRole("main");
  await waitFor(() => expect(document.activeElement).toBe(main));
};

beforeEach(() => {
  localStorage.clear();
  markSeen(BUILD);
  navigate("today");
  vi.stubGlobal("matchMedia", (q: string) => ({
    matches: false,
    media: q,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
  visibility = "visible";
  vi.spyOn(document, "visibilityState", "get").mockImplementation(
    () => visibility,
  );
});
afterEach(() => {
  clearMocks();
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

/** The shell on Today, its devices warm, every place's commands answered as one core would. */
function places(): UiBackend {
  const ui = installUi({
    devices: devicesOf({ trainer: neo }, {}, { trainer: "warm" }),
  });
  installToday({ others: invoke() });
  return ui;
}

describe("the route", () => {
  // §5.1.8 "Idle cool-down disconnects" (the core starts `T_cool` on `Left`) and "Every `Release` reason disconnects
  // at once" (Settings → Devices): what each view sends.
  test("warm views send Likely, Settings → Devices releases, every other view sends Left", async () => {
    const ui = places();
    render(<App readBuild={readBuild} />);
    await ready();
    await waitFor(() => expect(intents(ui.calls)).toEqual([LIKELY]));
    // One call at a time (intent.ts): each view waits for the answer to the last, so walk them one by one.
    const walk: [
      Parameters<typeof navigate>[0],
      string | undefined,
      LinkIntent | null,
    ][] = [
      // Workouts is warm as Today is: no change, nothing sent
      ["workouts", undefined, null],
      ["history", undefined, LEFT],
      ["today", undefined, LIKELY],
      ["settings", "appearance", LEFT],
      ["settings", "devices", { kind: "release", reason: "devices" }],
      ["settings", "athlete", LEFT],
    ];
    const expected: LinkIntent[] = [LIKELY];
    for (const [view, sub, intent] of walk) {
      act(() => navigate(view, sub));
      if (intent !== null) expected.push(intent);
      await waitFor(() => expect(intents(ui.calls)).toEqual(expected));
    }
    // the release let the keeper's trainer go (ui backend: keeper/mod.rs `Keeper::free_intent`)
    expect(ui.truth.devices.trainer.phase).toBe("cold");
  });

  test("Settings → Devices releases through the Devices view's own backend", async () => {
    const { ui } = installDevices();
    render(<App readBuild={readBuild} />);
    await ready();
    act(() => navigate("settings", "devices"));
    await screen.findByRole("heading", { name: "Devices" });
    await waitFor(() =>
      expect(intents(ui.calls)).toEqual([
        LIKELY,
        { kind: "release", reason: "devices" },
      ]),
    );
  });
});

describe("the Ready sheet", () => {
  // §5.1.8 "The Ready sheet holds": over a load that has not installed its ride, `Likely { ready: true }`.
  test("over a load in flight it sends Likely { ready: true }, then Likely once the ride is installed", async () => {
    const ui = places();
    render(<App readBuild={readBuild} />);
    await ready();
    act(() => ui.loadStarted());
    act(() => navigate("ride"));
    await waitFor(() =>
      expect(intents(ui.calls)).toEqual([
        LIKELY,
        { kind: "likely", ready: true },
      ]),
    );
    act(() => ui.installed(rideView()));
    await waitFor(() =>
      expect(intents(ui.calls)).toEqual([
        LIKELY,
        { kind: "likely", ready: true },
        LIKELY,
      ]),
    );
  });

  // §5.1.8 "A loaded ride keeps its links": hiding the window releases (`Hidden`), and the loaded, unstarted ride and
  // its links stay; "An abandoned load releases … Input at 9 min restarts the timer": rider input on the sheet is a
  // `Likely` the core counts for `T_abandon`.
  test("over a loaded ride: input refreshes, hiding releases and the ride keeps its links, showing sends Likely", async () => {
    const rig = installRide({ phase: "loaded" });
    navigate("ride");
    render(<App readBuild={readBuild} />);
    await screen.findByRole("button", { name: /Start/ });
    await waitFor(() => expect(intents(rig.calls)).toEqual([LIKELY]));
    // The refresh is at most once per 30 s, counted from the Likely just sent: a key now sends nothing.
    fireEvent.keyDown(window, { key: "a" });
    await act(async () => {});
    expect(intents(rig.calls)).toEqual([LIKELY]);
    setVisibility("hidden");
    await waitFor(() =>
      expect(intents(rig.calls)).toEqual([
        LIKELY,
        { kind: "release", reason: "hidden" },
      ]),
    );
    expect(rig.world.phase).toBe("loaded");
    expect(rig.ui.truth.devices.trainer.phase).toBe("riding");
    setVisibility("visible");
    await waitFor(() =>
      expect(intents(rig.calls)).toEqual([
        LIKELY,
        { kind: "release", reason: "hidden" },
        LIKELY,
      ]),
    );
  });

  // §5.1.8 "`Likely` during a ride warms nothing": a started ride is no warm view, so its screen sends `Left`, which
  // the keeper ignores while the ride holds the links; input sends nothing more.
  test("a started ride sends Left and input sends nothing", async () => {
    const rig = installRide({ phase: "active" });
    navigate("ride");
    render(<App readBuild={readBuild} />);
    await screen.findByRole("button", { name: /Pause/ });
    await waitFor(() => expect(intents(rig.calls)).toEqual([LEFT]));
    fireEvent.pointerDown(window);
    fireEvent.keyDown(window, { key: " " });
    await act(async () => {});
    expect(intents(rig.calls)).toEqual([LEFT]);
    expect(rig.world.phase).toBe("active");
  });
});
