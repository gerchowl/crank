// @vitest-environment jsdom
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { RideTiers } from "./useTypeTiers";
import * as settings from "./rideSettings";
import * as display from "./displayMm";

// The shell's answer and its monitor-change signal (#286): no Tauri here.
vi.mock("./displayMm", () => ({
  readDisplayWidthMm: vi.fn(async () => null),
  watchMonitorChange: vi.fn(() => () => undefined),
}));

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const setViewport = (w: number, h: number) => {
  vi.spyOn(window, "innerWidth", "get").mockReturnValue(w);
  vi.spyOn(window, "innerHeight", "get").mockReturnValue(h);
  window.dispatchEvent(new Event("resize"));
};

function laptopAt(w: number, h: number) {
  vi.spyOn(window.screen, "width", "get").mockReturnValue(1440);
  vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(1);
  vi.spyOn(settings, "useMonitorWidthMmOverride").mockReturnValue(288);
  setViewport(w, h);
}

test("shows the banner at T3+ size when T2 does not fit, and drops it on resize", async () => {
  laptopAt(1200, 700);
  render(
    <RideTiers>
      <p>ride</p>
    </RideTiers>,
  );
  const banner = await screen.findByRole("alert");
  expect(banner.closest("[data-slot]")!.getAttribute("data-slot")).toBe(
    "status-bar",
  );
  expect(banner.textContent).toBe("Window too small to read targets");
  expect(banner.style.fontSize).toBe("var(--t3p)");
  expect(
    banner
      .closest<HTMLElement>("[data-slot=status-bar]")!
      .parentElement!.style.getPropertyValue("--t3p"),
  ).toBe("40px");

  act(() => setViewport(1440, 900));
  await vi.waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
});

test("shows the unknown-size notice instead when the physical size is unknown", async () => {
  vi.spyOn(settings, "useMonitorWidthMmOverride").mockReturnValue(null);
  setViewport(1440, 900);
  render(<RideTiers>x</RideTiers>);
  expect((await screen.findByRole("status")).textContent).toMatch(
    /Screen size unknown/,
  );
  expect(screen.queryByRole("alert")).toBeNull();
});

test("showing or hiding the warning leaves the content's position and the slot unchanged", async () => {
  laptopAt(1440, 900);
  render(
    <RideTiers>
      <p>ride</p>
    </RideTiers>,
  );
  const root = (await screen.findByText("ride")).closest(
    "[data-slot=content]",
  )!.parentElement!;
  const content = root.children[0]!;
  const slot = root.children[1] as HTMLElement;
  const before = [slot.style.height, slot.style.flex, content.outerHTML];
  expect(screen.queryByRole("alert")).toBeNull();

  act(() => setViewport(1200, 700));
  await screen.findByRole("alert");
  expect(root.children[0]).toBe(content);
  expect(root.children[1]).toBe(slot);
  expect([slot.style.height, slot.style.flex, content.outerHTML]).toEqual(
    before,
  );
});

test("a devicePixelRatio change without a resize recomputes the tiers", async () => {
  const listeners: (() => void)[] = [];
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({
      addEventListener: (_: string, l: () => void) => listeners.push(l),
      removeEventListener: (_: string, l: () => void) => {
        const i = listeners.indexOf(l);
        if (i >= 0) listeners.splice(i, 1);
      },
    })),
  );
  laptopAt(1440, 900);
  const dpr = vi.spyOn(window, "devicePixelRatio", "get");
  dpr.mockReturnValue(1);
  render(<RideTiers>x</RideTiers>);
  const root = (await screen.findByText("x")).closest(
    "[data-slot=content]",
  )!.parentElement!;
  expect(root.style.getPropertyValue("--t2")).toBe("101px");

  // Moving to a 2× monitor of the same physical width: screen.width in CSS px halves.
  vi.spyOn(window.screen, "width", "get").mockReturnValue(720);
  dpr.mockReturnValue(2);
  act(() => listeners.slice().forEach((l) => l()));
  await vi.waitFor(() =>
    expect(root.style.getPropertyValue("--t2")).toBe("51px"),
  );
});

// ---- #286: the measured width from the shell ----

const rootOf = async () =>
  (await screen.findByText("x")).closest("[data-slot=content]")!.parentElement!;

test("a width from the shell gives measured tiers and no unknown-size notice", async () => {
  vi.spyOn(window.screen, "width", "get").mockReturnValue(1440);
  vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(1);
  vi.spyOn(settings, "useMonitorWidthMmOverride").mockReturnValue(null);
  vi.mocked(display.readDisplayWidthMm).mockResolvedValue(288);
  setViewport(1440, 900);
  render(<RideTiers>x</RideTiers>);
  const root = await rootOf();
  // the same tiers a 288 mm override gives (the dpr test above), not the clamp fallback
  await vi.waitFor(() =>
    expect(root.style.getPropertyValue("--t2")).toBe("101px"),
  );
  expect(screen.queryByRole("status")).toBeNull();
});

test("a move to another monitor refreshes the width and recomputes the tiers", async () => {
  vi.spyOn(window.screen, "width", "get").mockReturnValue(1440);
  vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(1);
  vi.spyOn(settings, "useMonitorWidthMmOverride").mockReturnValue(null);
  const read = vi.mocked(display.readDisplayWidthMm);
  read.mockClear(); // earlier tests' calls share this module mock
  read.mockResolvedValue(288);
  // Both the measured-width store and the screen snapshot subscribe (#390): a monitor change reaches them all.
  const watchers: (() => void)[] = [];
  const moved = () => watchers.slice().forEach((w) => w());
  vi.mocked(display.watchMonitorChange).mockImplementation((notify) => {
    watchers.push(notify);
    return () => undefined;
  });
  setViewport(1440, 900);
  render(<RideTiers>x</RideTiers>);
  const root = await rootOf();
  await vi.waitFor(() =>
    expect(root.style.getPropertyValue("--t2")).toBe("101px"),
  );

  // The window lands on a physically wider monitor with the same pixel size.
  read.mockResolvedValue(597);
  act(() => moved());
  await vi.waitFor(() =>
    expect(root.style.getPropertyValue("--t2")).not.toBe("101px"),
  );
  expect(read).toHaveBeenCalledTimes(2);
});

test("the settings override wins over the measured width", async () => {
  vi.spyOn(window.screen, "width", "get").mockReturnValue(1440);
  vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(1);
  vi.spyOn(settings, "useMonitorWidthMmOverride").mockReturnValue(288);
  vi.mocked(display.readDisplayWidthMm).mockResolvedValue(597);
  setViewport(1440, 900);
  render(<RideTiers>x</RideTiers>);
  const root = await rootOf();
  await vi.waitFor(() => expect(display.readDisplayWidthMm).toHaveBeenCalled());
  await act(async () => undefined);
  expect(root.style.getPropertyValue("--t2")).toBe("101px");
});

// ---- #313: reads are ordered ----

test("a slow earlier read cannot overwrite a newer one (the review's repro)", async () => {
  vi.spyOn(window.screen, "width", "get").mockReturnValue(1440);
  vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(1);
  vi.spyOn(settings, "useMonitorWidthMmOverride").mockReturnValue(null);
  const answers: ((v: number | null) => void)[] = [];
  const read = vi.mocked(display.readDisplayWidthMm);
  read.mockClear();
  read.mockImplementation(
    () => new Promise<number | null>((resolve) => answers.push(resolve)),
  );
  // Both the measured-width store and the screen snapshot subscribe (#390): a monitor change reaches them all.
  const watchers: (() => void)[] = [];
  const moved = () => watchers.slice().forEach((w) => w());
  vi.mocked(display.watchMonitorChange).mockImplementation((notify) => {
    watchers.push(notify);
    return () => undefined;
  });
  setViewport(1440, 900);
  render(<RideTiers>x</RideTiers>);
  const root = await rootOf();
  await vi.waitFor(() => expect(answers).toHaveLength(1)); // the mount read, held
  act(() => moved()); // the window moved: the second read
  await vi.waitFor(() => expect(answers).toHaveLength(2));
  await act(async () => answers[1]!(597)); // the newer read answers first: 49px
  await vi.waitFor(() =>
    expect(root.style.getPropertyValue("--t2")).toBe("49px"),
  );
  await act(async () => answers[0]!(288)); // then the slow mount read: must not win (it would give 101px)
  expect(root.style.getPropertyValue("--t2")).toBe("49px");
});

// ---- #390: a same-scale "Looks like" change ----

test("a screen.width change at the same scale recomputes the tiers on the monitor-change signal", async () => {
  const watchers: (() => void)[] = [];
  vi.mocked(display.watchMonitorChange).mockImplementation((notify) => {
    watchers.push(notify);
    return () => undefined;
  });
  const screenW = vi.spyOn(window.screen, "width", "get");
  screenW.mockReturnValue(1440);
  vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(2);
  vi.spyOn(settings, "useMonitorWidthMmOverride").mockReturnValue(288);
  setViewport(1440, 900);
  render(<RideTiers>x</RideTiers>);
  const root = await rootOf();
  const before = root.style.getPropertyValue("--t2");

  // "Looks like" 1440 -> 1920: same dpr, no resize, no dpr media-query change.
  screenW.mockReturnValue(1920);
  act(() => watchers.slice().forEach((w) => w()));
  await vi.waitFor(() =>
    expect(root.style.getPropertyValue("--t2")).not.toBe(before),
  );
});
