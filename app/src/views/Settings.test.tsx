// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test } from "vitest";
import { Settings } from "./Settings";

afterEach(cleanup);

describe("Settings (DESIGN §5.9)", () => {
  test("the Athlete model section is listed between Devices and Appearance", () => {
    render(<Settings />);
    const nav = screen.getByRole("navigation", { name: "Settings sections" });
    expect(
      [...nav.querySelectorAll("button")].map((b) => b.textContent),
    ).toEqual(["Devices", "Athlete model", "Appearance"]);
  });
});
