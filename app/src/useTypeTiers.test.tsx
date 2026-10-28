// @vitest-environment jsdom
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { RideTiers } from "./useTypeTiers";
import * as settings from "./rideSettings";

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
