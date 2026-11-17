// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, expect, test, vi } from "vitest";
import { AppRoot } from "./App";
import { markSeen } from "./build";
import { navigate } from "./router";
import { install as installToday } from "./views/todayBackend";

let broken = true;
vi.mock("./shell/Rail", () => ({
  Rail: () => {
    if (broken) throw new Error("rail exploded");
    return <nav aria-label="Places" />;
  },
}));

afterEach(() => {
  clearMocks();
  cleanup();
});

test("a throw outside the routed view (the rail) still gets the alert panel and a way back", async () => {
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  navigate("today");
  // The way back lands on Today, which reads the core.
  installToday();
  // A launch whose spin-up has played before: the hand-off is not held (App.test.tsx covers the hold).
  markSeen("0.2.0 · 7516045");
  render(<AppRoot readBuild={() => Promise.resolve("0.2.0 · 7516045")} />);
  expect((await screen.findByRole("alert")).textContent).toMatch(
    /rail exploded/,
  );
  broken = false;
  await userEvent
    .setup()
    .click(screen.getByRole("button", { name: "Back to Today" }));
  expect(screen.queryByRole("alert")).toBeNull();
  expect(
    await screen.findByRole("navigation", { name: "Places" }),
  ).toBeTruthy();
  err.mockRestore();
});
