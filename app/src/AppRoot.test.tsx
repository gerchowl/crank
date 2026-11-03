// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import { AppRoot } from "./App";
import { navigate } from "./router";

let broken = true;
vi.mock("./shell/Rail", () => ({
  Rail: () => {
    if (broken) throw new Error("rail exploded");
    return <nav aria-label="Places" />;
  },
}));

afterEach(cleanup);

test("a throw outside the routed view (the rail) still gets the alert panel and a way back", async () => {
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  navigate("today");
  render(<AppRoot />);
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
