// @vitest-environment jsdom
// The live-data notice (ChannelNotice.tsx): the stale channel and a broken frame are shown on every place but the ride
// screen, and go with the next frame.
import { act, cleanup, render, screen } from "@testing-library/react";
import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, expect, test, vi } from "vitest";
import { UiClient, WATCHDOG_MS, type Timers } from "../stream/ui";
import { installUi, rideView } from "../stream/uiBackend";
import { UiProvider } from "../stream/uiHooks";
import { ChannelNotice } from "./ChannelNotice";

afterEach(() => {
  cleanup();
  clearMocks();
  vi.restoreAllMocks();
});

/** A clock the test fires by hand. */
const manual = () => {
  let due: (() => void) | null = null;
  const timers: Timers = {
    setTimeout: (fn) => {
      due = fn;
      return 1;
    },
    clearTimeout: () => {
      due = null;
    },
  };
  return { timers, fire: () => due?.() };
};
const flush = () =>
  act(async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  });

test("a channel silent for 2 s is shown as stale until the next frame", async () => {
  vi.spyOn(console, "warn").mockImplementation(() => {});
  const ui = installUi({ ride: rideView() });
  const clock = manual();
  const client = new UiClient(clock.timers);
  render(
    <UiProvider client={client}>
      <ChannelNotice />
    </UiProvider>,
  );
  await flush();
  expect(screen.queryByTestId("channel-stale")).toBeNull();
  ui.stall();
  act(() => clock.fire());
  expect(screen.getByRole("alert").textContent).toBe(
    "⚠ Live data stopped: what is shown may be out of date. Reconnecting…",
  );
  ui.resume();
  await flush();
  expect(screen.queryByTestId("channel-stale")).toBeNull();
  expect(WATCHDOG_MS).toBe(2000);
});

test("a broken frame in production is shown with the fault, and goes with the repair", async () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  const ui = installUi({ ride: rideView() });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  const clock = manual();
  const client = new UiClient(clock.timers, undefined, false);
  render(
    <UiProvider client={client}>
      <ChannelNotice />
    </UiProvider>,
  );
  await flush();
  const channel = ui.calls.find((c) => c.cmd === "ui_open")?.args.channel as {
    onmessage: (f: unknown) => void;
  };
  act(() =>
    channel.onmessage({
      ...ui.frames[0],
      seq: 1,
      series: {
        ...ui.frames[0].series,
        hr: { from: 2, t: [1], w: [1], v: [120] },
      },
    }),
  );
  expect(screen.getByTestId("channel-stale").textContent).toContain(
    "The core sent a broken update (hr: from 2 on a series of 0).",
  );
  // Not acked: the watchdog reopens and the backfill repairs it.
  ui.resume();
  act(() => clock.fire());
  await flush();
  expect(screen.queryByTestId("channel-stale")).toBeNull();
});

test("a refused ui_open is shown on Workouts, History and Settings with Retry, not where the view shows it", async () => {
  const { navigate } = await import("../router");
  const ui = installUi({ keeperStorage: true });
  const client = new UiClient(manual().timers);
  render(
    <UiProvider client={client}>
      <ChannelNotice />
    </UiProvider>,
  );
  for (const [view, sub] of [
    ["workouts", undefined],
    ["history", undefined],
    ["settings", undefined],
  ] as const) {
    act(() => navigate(view, sub));
    await flush();
    expect(screen.getByTestId("channel-refused").textContent).toBe(
      "⚠ Live data is not available: database is locked. Trying again every 2 s.",
    );
  }
  // Today and Settings → Devices show it beside the devices it hides.
  act(() => navigate("today"));
  expect(screen.queryByTestId("channel-refused")).toBeNull();
  act(() => navigate("settings", "devices"));
  expect(screen.queryByTestId("channel-refused")).toBeNull();
  // Retry opens the channel at once.
  act(() => navigate("workouts"));
  ui.truth.keeperStorage = false;
  act(() => screen.getByRole("button", { name: "Retry" }).click());
  await flush();
  expect(screen.queryByTestId("channel-refused")).toBeNull();
  expect(ui.count("ui_open")).toBe(2);
});
