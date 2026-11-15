// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test } from "vitest";
import { setAthleteHr } from "../athleteHr";
import { NO_HR_REFS, Settings } from "./Settings";

afterEach(() => {
  cleanup();
  setAthleteHr(null);
});

describe('Settings → Athlete model (DESIGN §5.9, §18.3 "Heart-rate zone", #586)', () => {
  test("neither HR reference known: both say Not set and the one-line hint shows", () => {
    render(<Settings sub="athlete" />);
    expect(screen.getByRole("heading", { name: "Athlete model" })).toBeTruthy();
    expect(screen.getByTestId("hrmax").textContent).toBe("Not set");
    expect(screen.getByTestId("lthr").textContent).toBe("Not set");
    expect(screen.getByText(NO_HR_REFS)).toBeTruthy();
  });

  test.each([
    [{ lthrBpm: 165 }, "Not set", "165 bpm"],
    [{ hrMaxBpm: 190 }, "190 bpm", "Not set"],
  ])("with %o the values show and the hint does not", (refs, max, lthr) => {
    setAthleteHr(refs);
    render(<Settings sub="athlete" />);
    expect(screen.getByTestId("hrmax").textContent).toBe(max);
    expect(screen.getByTestId("lthr").textContent).toBe(lthr);
    expect(screen.queryByText(NO_HR_REFS)).toBeNull();
  });

  test("the section is listed between Devices and Appearance", () => {
    render(<Settings />);
    const nav = screen.getByRole("navigation", { name: "Settings sections" });
    expect(
      [...nav.querySelectorAll("button")].map((b) => b.textContent),
    ).toEqual(["Devices", "Athlete model", "Appearance"]);
  });
});
