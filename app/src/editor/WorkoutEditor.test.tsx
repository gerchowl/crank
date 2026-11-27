// @vitest-environment jsdom
// The editor's three views stay in sync through the model (#111): the outline, the text and the profile show one
// state, undo/redo spans them by button and by Cmd/Ctrl+Z / Shift+Cmd/Ctrl+Z (only while focus is inside the editor),
// and an invalid draft leaves the saved state alone. On the workouts backend, which answers the editor's commands only
// with the core's own answers (editorFixture.ts).
import {
  act,
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, test } from "vitest";
import { Rail } from "../shell/Rail";
import { navigate } from "../router";
import { Workouts } from "../views/Workouts";
import { DRAFTS } from "../views/editorFixture";
import { resetRideLoad } from "../views/rideLoad";
import { apiError, install } from "../views/workoutsBackend";
import { GROUP_MS } from "./model";
import { setText as setViewText, textOf } from "./TextView";
import { lineColumn, typingClock } from "./WorkoutEditor";
import { windowClose } from "./windowClose";

afterEach(() => {
  cleanup();
  clearMocks();
});
/** Each keystroke a pause after the last, so each text is its own undo step unless a test says otherwise. */
let now = 0;
beforeEach(() => {
  resetRideLoad();
  navigate("workouts");
  now = 0;
  typingClock.now = () => (now += GROUP_MS);
  // No Tauri window here: the close-request listener of an editor with unsaved changes is held by nobody.
  windowClose.listen = () => Promise.resolve(() => {});
  windowClose.destroy = () => Promise.resolve();
});
const text = (name: string) => DRAFTS[name]!.text;
/** Replace the whole text, as a paste would: one transaction from the rider (TextView.tsx `setText`). */
const setText = (box: HTMLElement, t: string) => act(() => setViewText(box, t));
const problemOf = (name: string) => {
  const d = DRAFTS[name]!;
  if (!("problem" in d)) throw new Error(`${name} parses`);
  return d.problem;
};

/** Workouts with "Sweet spot 3x12" as the core's `sweet-spot` draft, opened in the editor. */
async function open() {
  const b = install({ trees: { "w-ss": "sweet-spot" } });
  const user = userEvent.setup();
  render(
    <>
      <Rail view="workouts" />
      <Workouts />
    </>,
  );
  await user.click(
    await screen.findByRole("option", { name: /Sweet spot 3x12/ }),
  );
  await user.click(
    await screen.findByRole("button", { name: "Duplicate & edit" }),
  );
  const box = await screen.findByRole("textbox", { name: "Text" });
  return { b, user, box: box as HTMLElement };
}
const outline = () => screen.getByRole("listbox", { name: "Outline" });
const rows = () =>
  within(outline())
    .getAllByRole("option")
    .map((o) => o.textContent);

test("Duplicate & edit opens a copy with its three views; Back returns to the library", async () => {
  const { user, box } = await open();
  expect(screen.getByRole("heading", { level: 1 }).textContent).toContain(
    "Sweet spot 3x12 (copy)",
  );
  expect(screen.getByText("Not saved")).toBeTruthy();
  expect(textOf(box)).toBe(text("sweet-spot"));
  expect(rows()).toEqual([
    "Warm-up · 10:00 · ramp 45–70 %",
    "3 ×",
    "in the repeat above, Sweet spot · 12:00 · 90 %",
    "in the repeat above, Rest · 4:00 · 55 %",
    "Cool-down · 5:00 · ramp 60–40 %",
  ]);
  expect(screen.getByRole("img", { name: /^Profile: 8 blocks/ })).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Back to library" }));
  expect(
    await screen.findByRole("option", {
      name: /Sweet spot 3x12/,
      selected: true,
    }),
  ).toBeTruthy();
});

test("typing a valid text updates the outline, the profile and the summary", async () => {
  const { b, box } = await open();
  setText(box, text("sweet-spot-4x"));
  await waitFor(() => expect(rows()[1]).toBe("4 ×"));
  await screen.findByRole("img", { name: /^Profile: 10 blocks/ });
  expect(b.calls.some((c) => c.cmd === "compile_workout")).toBe(true);
  // The preview names the FTP the core compiled it at: the athlete model's (ADR-0009 §2.1.4).
  const summary = screen.getByRole("region", { name: "Compiled summary" });
  expect(within(summary).getByTestId("compiled-ftp").textContent).toBe(
    "At FTP 250 W",
  );
});

test("an invalid draft shows the problem and leaves the saved state, the outline and the profile alone", async () => {
  const { box } = await open();
  const before = rows();
  setText(box, text("missing-target"));
  const problem = await screen.findByText(/Missing power target/);
  const { line, column } = lineColumn(
    text("missing-target"),
    problemOf("missing-target").offset,
  );
  expect(problem.textContent).toContain(
    `(line ${line}, column ${column}). The saved workout is unchanged.`,
  );
  expect(box.getAttribute("aria-invalid")).toBe("true");
  // The problem's line, its span (the core's offset and length) marked.
  expect(problem.querySelector("mark")?.textContent).toBe(
    text("missing-target").slice(
      problemOf("missing-target").offset,
      problemOf("missing-target").offset + problemOf("missing-target").length,
    ),
  );
  expect(textOf(box)).toBe(text("missing-target"));
  expect(rows()).toEqual(before);
  expect(
    (screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
});

test("undo and redo span the three views, by button and by Cmd/Ctrl+Z inside the editor only", async () => {
  const { user, box } = await open();
  setText(box, text("sweet-spot-4x"));
  await waitFor(() => expect(rows()[1]).toBe("4 ×"));
  setText(box, text("sweet-spot-4x-15m"));
  await waitFor(() => expect(rows()[2]).toMatch(/15:00/));

  await user.click(screen.getByRole("button", { name: "Undo" }));
  expect(textOf(box)).toBe(text("sweet-spot-4x"));
  expect(rows()[2]).toMatch(/12:00/);

  box.focus();
  await user.keyboard("{Meta>}z{/Meta}");
  expect(textOf(box)).toBe(text("sweet-spot"));
  expect(rows()[1]).toBe("3 ×");
  await screen.findByRole("img", { name: /^Profile: 8 blocks/ });

  await user.keyboard("{Control>}{Shift>}z{/Shift}{/Control}");
  expect(textOf(box)).toBe(text("sweet-spot-4x"));
  expect(rows()[1]).toBe("4 ×");

  // Outside the editor (focus on a rail button) the keys do nothing.
  const rail = screen.getByRole("button", { name: /History/ });
  rail.focus();
  expect(document.activeElement).toBe(rail);
  await user.keyboard("{Meta>}z{/Meta}");
  expect(textOf(box)).toBe(text("sweet-spot-4x"));
  expect(rows()[1]).toBe("4 ×");
});

test("a burst of typing is one undo step; a pause starts the next", async () => {
  const { user, box } = await open();
  typingClock.now = () => (now += GROUP_MS / 5);
  setText(box, text("sweet-spot-4x"));
  await waitFor(() => expect(rows()[1]).toBe("4 ×"));
  setText(box, text("sweet-spot-4x-15m"));
  await waitFor(() => expect(rows()[2]).toMatch(/15:00/));
  typingClock.now = () => (now += GROUP_MS);
  setText(box, text("sweet-spot-no-cool-down"));
  await waitFor(() => expect(rows()).toHaveLength(4));

  await user.click(screen.getByRole("button", { name: "Undo" }));
  expect(textOf(box)).toBe(text("sweet-spot-4x-15m"));
  await user.click(screen.getByRole("button", { name: "Undo" }));
  expect(textOf(box)).toBe(text("sweet-spot"));
  expect(
    (screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
});

test("the profile marks the selected node's blocks; the selection survives undo while its node exists, else returns", async () => {
  const { user, box } = await open();
  await user.click(
    within(outline()).getByRole("option", { name: /Cool-down/ }),
  );
  const marked = () =>
    document.querySelectorAll(".workouts-profile-block[data-marked]");
  expect(marked()).toHaveLength(1);
  expect(screen.getByRole("img", { name: /, 1 marked$/ })).toBeTruthy();

  await user.click(within(outline()).getByRole("option", { name: /3 ×/ }));
  expect(marked()).toHaveLength(6); // the repeat's three work and three rest blocks
  await user.keyboard("{ArrowDown}");
  expect(
    within(outline()).getByRole("option", { selected: true }).textContent,
  ).toMatch(/Sweet spot/);
  expect(marked()).toHaveLength(3);

  // A text commit gives fresh ids; the selection follows its path, then undo keeps it while the node exists.
  setText(box, text("sweet-spot-4x"));
  await waitFor(() => expect(rows()[1]).toBe("4 ×"));
  expect(
    within(outline()).getByRole("option", { selected: true }).textContent,
  ).toMatch(/Sweet spot/);
  await user.click(
    within(outline()).getByRole("option", { name: /Cool-down/ }),
  );
  setText(box, text("sweet-spot-no-cool-down"));
  await waitFor(() => expect(rows()).toHaveLength(4));
  expect(
    within(outline()).queryByRole("option", { selected: true }),
  ).toBeNull();
  // Undo brings Cool-down back, selected again: the step was left with it selected (#653).
  await user.click(screen.getByRole("button", { name: "Undo" }));
  expect(
    within(outline()).getByRole("option", { selected: true }).textContent,
  ).toMatch(/Cool-down/);
});

test("a failed parse shows its error until the next parse answers, even one that commits nothing", async () => {
  const { b, box } = await open();
  b.fail.parse_dsl = apiError("internal", "core gone");
  setText(box, text("sweet-spot-4x"));
  expect((await screen.findByRole("alert")).textContent).toContain("core gone");
  delete b.fail.parse_dsl;
  setText(box, text("sweet-spot")); // the saved text: the answer commits nothing
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
});

test("a draft the core cannot compile shows its refusal; the last compiled view stays", async () => {
  const { box } = await open();
  setText(box, text("too-long"));
  expect((await screen.findByRole("alert")).textContent).toContain(
    "workout does not compile: TraceLimit",
  );
  expect(screen.getByRole("img", { name: /^Profile: 8 blocks/ })).toBeTruthy();
});

test("Edit ▸ Undo and Redo reach the text as beforeinput (WebKit's menu path) and undo in all three views", async () => {
  const { box } = await open();
  setText(box, text("sweet-spot-4x"));
  await waitFor(() => expect(rows()[1]).toBe("4 ×"));
  const menu = (inputType: string) => {
    const e = new InputEvent("beforeinput", {
      inputType,
      bubbles: true,
      cancelable: true,
    });
    act(() => void box.dispatchEvent(e));
    return e.defaultPrevented;
  };
  expect(menu("historyUndo")).toBe(true);
  expect(textOf(box)).toBe(text("sweet-spot"));
  expect(rows()[1]).toBe("3 ×");
  expect(menu("historyRedo")).toBe(true);
  expect(textOf(box)).toBe(text("sweet-spot-4x"));
  expect(rows()[1]).toBe("4 ×");
});

test("an undo is shown in the text without being read back: the text sends no edit for it", async () => {
  const { b, user, box } = await open();
  setText(box, text("sweet-spot-4x"));
  await waitFor(() => expect(rows()[1]).toBe("4 ×"));
  const parses = () => b.calls.filter((c) => c.cmd === "parse_dsl").length;
  const before = parses();
  await user.click(screen.getByRole("button", { name: "Undo" }));
  await user.click(screen.getByRole("button", { name: "Redo" }));
  expect(textOf(box)).toBe(text("sweet-spot-4x"));
  expect(parses()).toBe(before);
  // Nor did it start a burst: the next text is a step of its own, so one Undo goes back to the redone text.
  setText(box, text("sweet-spot-4x-15m"));
  await waitFor(() => expect(rows()[2]).toMatch(/15:00/));
  await user.click(screen.getByRole("button", { name: "Undo" }));
  expect(textOf(box)).toBe(text("sweet-spot-4x"));
});

test("Tab and Shift+Tab move focus out of the text box (no keyboard trap)", async () => {
  const { user, box } = await open();
  act(() => box.focus());
  expect(document.activeElement).toBe(box);
  await user.tab();
  expect(document.activeElement).not.toBe(box);
  expect(textOf(box)).toBe(text("sweet-spot"));
  act(() => box.focus());
  await user.tab({ shift: true });
  expect(document.activeElement).not.toBe(box);
  expect(textOf(box)).toBe(text("sweet-spot"));
});
