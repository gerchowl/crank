// @vitest-environment jsdom
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { RideTiers } from "./useTypeTiers";
import * as settings from "./rideSettings";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
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
  expect(banner.textContent).toBe("Window too small to read targets");
  expect(banner.style.fontSize).toBe("var(--t3p)");
  expect(banner.parentElement!.style.getPropertyValue("--t3p")).toBe("40px");

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
