// @vitest-environment jsdom
// Editing the tree from the outline (#653, DESIGN §5.7 "Outline editing"): each toolbar action and each form edit
// commits the core's canonical text as exactly one undo step (undo and redo restore it), by pointer and by keyboard,
// with focus on the edited row; an edit the core cannot read back is refused with its message and changes nothing; a
// built-in is changed only through its copy. On the workouts backend, which answers `print_dsl` and `parse_dsl` only
// with the core's own answers (editorFixture.ts, crates/crank-core/tests/fixtures/editor.json).
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
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { Rail } from "../shell/Rail";
import { navigate } from "../router";
import { Workouts } from "../views/Workouts";
import { DRAFTS } from "../views/editorFixture";
import { resetRideLoad } from "../views/rideLoad";
import { install, type Backend } from "../views/workoutsBackend";
import type { WorkoutNode } from "../bindings";
import { GROUP_MS } from "./model";
import { nodeFrom, outlineActions, soleChildOf } from "./OutlineEdit";
import { setText as setViewText, textOf } from "./TextView";
import { typingClock } from "./WorkoutEditor";
import { windowClose } from "./windowClose";

afterEach(() => {
  cleanup();
  clearMocks();
});
let now = 0;
beforeEach(() => {
  resetRideLoad();
  navigate("workouts");
  now = 0;
  typingClock.now = () => (now += GROUP_MS);
  windowClose.listen = () => Promise.resolve(() => {});
  windowClose.destroy = () => Promise.resolve();
});
const text = (name: string) => DRAFTS[name]!.text;
/** Replace the whole text, as a paste would: one transaction from the rider (TextView.tsx `setText`). */
const setText = (box: HTMLElement, t: string) => act(() => setViewText(box, t));
const problemOf = (name: string) => {
  const d = DRAFTS[name]!;
  if (!("problem" in d)) throw new Error(`${name} parses`);
  return d.problem.message;
};

/** Workouts with both workouts as the core's `sweet-spot` draft; the built-in's copy, or the rider's own, opened. */
async function open(how: "copy" | "own" = "copy") {
  const b = install({
    trees: { "w-ss": "sweet-spot", "w-easy": "sweet-spot" },
  });
  const user = userEvent.setup();
  render(
    <>
      <Rail view="workouts" />
      <Workouts />
    </>,
  );
  const name = how === "copy" ? /Sweet spot 3x12/ : /Easy spin/;
  await user.click(await screen.findByRole("option", { name }));
  await user.click(
    await screen.findByRole("button", {
      name: how === "copy" ? "Duplicate & edit" : "Edit",
    }),
  );
  const box = (await screen.findByRole("textbox", {
    name: "Text",
  })) as HTMLElement;
  return { b, user, box };
}
const outline = () => screen.getByRole("listbox", { name: "Outline" });
const row = (name: RegExp) => within(outline()).getByRole("option", { name });
const toolbar = () => screen.getByRole("toolbar", { name: "Outline actions" });
const tool = (name: string) => within(toolbar()).getByRole("button", { name });
const undoButton = () => screen.getByRole("button", { name: "Undo" });
const redoButton = () => screen.getByRole("button", { name: "Redo" });
const calls = (b: Backend, cmd: string) => b.calls.filter((c) => c.cmd === cmd);

/** Exactly one undo step back to `before`, and redo forward to `after`, each with its own text. */
async function oneUndoStep(
  user: ReturnType<typeof userEvent.setup>,
  box: HTMLElement,
  before: string,
  after: string,
) {
  await user.click(undoButton());
  await waitFor(() => expect(textOf(box)).toBe(before));
  expect((undoButton() as HTMLButtonElement).disabled).toBe(true);
  await user.click(redoButton());
  await waitFor(() => expect(textOf(box)).toBe(after));
  expect((redoButton() as HTMLButtonElement).disabled).toBe(true);
}

/** Toolbar actions on the `sweet-spot` draft: the row, the button, the core's draft for the result, the edited row. */
const ACTIONS: [RegExp, string, string, RegExp][] = [
  [/Cool-down/, "Delete", "sweet-spot-no-cool-down", /Rest/],
  [/Warm-up/, "Duplicate", "dup-warm-up", /Warm-up/],
  [/Cool-down/, "Add step", "add-step", /^5:00 · 50 %$/],
  [/Cool-down/, "Wrap in repeat", "wrap-cool-down", /^2 ×$/],
  [/^3 ×$/, "Unwrap", "unwrapped", /Sweet spot/],
  [/Cool-down/, "Move up", "cool-down-up", /Cool-down/],
  [/^3 ×$/, "Move down", "cool-down-up", /^3 ×$/],
];

describe("toolbar actions", () => {
  test.each(ACTIONS)(
    "on %s, %s commits the core's text as one undo step, the edited row focused",
    async (on, action, draft, edited) => {
      const { user, box } = await open();
      await user.click(row(on));
      await user.click(tool(action));
      await waitFor(() => expect(textOf(box)).toBe(text(draft)));
      const focused = document.activeElement as HTMLElement;
      expect(focused.getAttribute("role")).toBe("option");
      expect(focused.getAttribute("aria-selected")).toBe("true");
      expect(focused.textContent).toMatch(edited);
      await oneUndoStep(user, box, text("sweet-spot"), text(draft));
    },
  );

  test.each(ACTIONS)(
    "by keyboard: on %s, %s from the toolbar (Shift+Tab from the row, arrows, Enter)",
    async (on, action, draft, edited) => {
      const { user, box } = await open();
      // Into the outline by keyboard: its tab stop is its first row; ↓ walks to the row (selection follows focus).
      within(outline()).getAllByRole("option")[0]!.focus();
      while (!(document.activeElement as HTMLElement).textContent!.match(on))
        await user.keyboard("{ArrowDown}");
      await user.keyboard("{Shift>}{Tab}{/Shift}");
      expect(document.activeElement?.closest("[role=toolbar]")).toBe(toolbar());
      await user.keyboard("{Home}");
      while (document.activeElement !== tool(action))
        await user.keyboard("{ArrowRight}");
      await user.keyboard("{Enter}");
      await waitFor(() => expect(textOf(box)).toBe(text(draft)));
      expect((document.activeElement as HTMLElement).textContent).toMatch(
        edited,
      );
      await oneUndoStep(user, box, text("sweet-spot"), text(draft));
    },
  );

  test("an action that does not apply to the selected row is disabled; the toolbar is one tab stop", async () => {
    const { user } = await open();
    await user.click(row(/Warm-up/));
    const disabled = within(toolbar())
      .getAllByRole("button")
      .filter((b) => (b as HTMLButtonElement).disabled)
      .map((b) => b.textContent);
    // A step is no repeat, and the first row cannot move up.
    expect(disabled).toEqual(["Unwrap", "Move up"]);
    const stops = within(toolbar())
      .getAllByRole("button")
      .filter((b) => b.tabIndex === 0);
    expect(stops).toHaveLength(1);
  });
});

describe("the selected node's form", () => {
  test("a repeat's count, then a step's minutes: each one undo step", async () => {
    const { user, box } = await open();
    await user.click(row(/^3 ×$/));
    const count = screen.getByRole("textbox", { name: "Count" });
    await user.clear(count);
    await user.type(count, "4");
    await user.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(textOf(box)).toBe(text("sweet-spot-4x")));
    // The form edits in place: focus stays in it, the row stays selected.
    expect(row(/^4 ×$/).getAttribute("aria-selected")).toBe("true");
    await user.click(row(/Sweet spot/));
    const min = screen.getByRole("textbox", { name: "Minutes" });
    await user.clear(min);
    // Enter in a field applies, as the form's submit.
    await user.type(min, "15{Enter}");
    await waitFor(() => expect(textOf(box)).toBe(text("sweet-spot-4x-15m")));
    await user.click(undoButton());
    await waitFor(() => expect(textOf(box)).toBe(text("sweet-spot-4x")));
    // The form shows the restored node's values.
    expect(
      (screen.getByRole("textbox", { name: "Minutes" }) as HTMLInputElement)
        .value,
    ).toBe("12");
    await user.click(undoButton());
    await waitFor(() => expect(textOf(box)).toBe(text("sweet-spot")));
    expect((undoButton() as HTMLButtonElement).disabled).toBe(true);
  });

  test.each([
    [/Sweet spot/, { Power: "95" }, "sweet-spot-95"],
    [/Warm-up/, { "Ramp from": "50", "Ramp to": "75" }, "warm-up-50-75"],
    [/Rest/, { Label: "Recover" }, "rest-recover"],
  ] as [RegExp, Record<string, string>, string][])(
    "on %s, %o commits the core's text as one undo step",
    async (on, values, draft) => {
      const { user, box } = await open();
      await user.click(row(on));
      for (const [label, value] of Object.entries(values)) {
        const f = screen.getByRole("textbox", { name: label });
        await user.clear(f);
        await user.type(f, value);
      }
      await user.click(screen.getByRole("button", { name: "Apply" }));
      await waitFor(() => expect(textOf(box)).toBe(text(draft)));
      await oneUndoStep(user, box, text("sweet-spot"), text(draft));
    },
  );

  test("by keyboard: Tab from the row into the form, type, Enter", async () => {
    const { user, box } = await open();
    within(outline()).getAllByRole("option")[0]!.focus();
    await user.keyboard("{ArrowDown}"); // the repeat
    await user.tab(); // Label
    await user.tab(); // Count
    expect(document.activeElement).toBe(
      screen.getByRole("textbox", { name: "Count" }),
    );
    await user.keyboard("{Control>}a{/Control}4{Enter}");
    await waitFor(() => expect(textOf(box)).toBe(text("sweet-spot-4x")));
  });

  test("Apply with nothing changed commits nothing", async () => {
    const { b, user } = await open();
    await user.click(row(/Rest/));
    await user.click(screen.getByRole("button", { name: "Apply" }));
    expect(calls(b, "print_dsl")).toEqual([]);
    expect((undoButton() as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("refused edits", () => {
  test.each([
    [/Warm-up/, "Minutes", "0", "zero-duration"],
    [/Sweet spot/, "Power", "-10", "negative-power"],
  ])(
    "on %s, %s %s is refused with the core's message; nothing changes",
    async (on, label, value, tree) => {
      const { user, box } = await open();
      await user.click(row(on));
      const f = screen.getByRole("textbox", { name: label });
      await user.clear(f);
      await user.type(f, value);
      await user.click(screen.getByRole("button", { name: "Apply" }));
      const refused = await screen.findByText(problemOf(tree), {
        exact: false,
      });
      expect(refused.id).toBe("editor-outline-refused");
      expect(refused.textContent).toContain("Nothing was changed.");
      expect(textOf(box)).toBe(text("sweet-spot"));
      expect((undoButton() as HTMLButtonElement).disabled).toBe(true);
      // What was typed stays, to be corrected.
      expect(
        (screen.getByRole("textbox", { name: label }) as HTMLInputElement)
          .value,
      ).toBe(value);
    },
  );

  test("a value that is no number is refused before the core is asked", async () => {
    const { b, user, box } = await open();
    await user.click(row(/Sweet spot/));
    const f = screen.getByRole("textbox", { name: "Power" });
    await user.clear(f);
    await user.type(f, "lots");
    await user.click(screen.getByRole("button", { name: "Apply" }));
    expect(
      (await screen.findByText(/Power: enter a number\./)).textContent,
    ).toContain("Nothing was changed.");
    expect(calls(b, "print_dsl")).toEqual([]);
    expect(textOf(box)).toBe(text("sweet-spot"));
  });

  test("while the text does not parse, the outline cannot be edited, and says why", async () => {
    const { user, box } = await open();
    await user.click(row(/Rest/));
    // The whole text at once, as a paste would (one change event).
    setText(box, text("missing-target"));
    await screen.findByText("Fix the text to edit the outline.");
    for (const b of within(toolbar()).getAllByRole("button"))
      expect((b as HTMLButtonElement).disabled).toBe(true);
    expect(
      (screen.getByRole("button", { name: "Apply" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });
});

describe("drafts and the library", () => {
  test("a new workout: Add step is its first step; the other actions wait for a selection", async () => {
    install({ trees: {} });
    const user = userEvent.setup();
    render(
      <>
        <Rail view="workouts" />
        <Workouts />
      </>,
    );
    await user.click(
      await screen.findByRole("button", { name: "New workout" }),
    );
    const box = (await screen.findByRole("textbox", {
      name: "Text",
    })) as HTMLElement;
    await screen.findByText("No steps yet: add one, or write one in Text.");
    const enabled = within(toolbar())
      .getAllByRole("button")
      .filter((b) => !(b as HTMLButtonElement).disabled)
      .map((b) => b.textContent);
    expect(enabled).toEqual(["Add step"]);
    await user.click(tool("Add step"));
    await waitFor(() => expect(textOf(box)).toBe(text("one-step")));
    expect(document.activeElement).toBe(row(/5:00 · 50 %/));
    await oneUndoStep(user, box, "", text("one-step"));
  });

  test("an outline edit of the rider's workout is an unsaved change, as typing is", async () => {
    const { user, box } = await open("own");
    expect(screen.getByText("Saved")).toBeTruthy();
    await user.click(row(/Cool-down/));
    await user.click(tool("Delete"));
    await waitFor(() =>
      expect(textOf(box)).toBe(text("sweet-spot-no-cool-down")),
    );
    expect(screen.getByText("Unsaved changes")).toBeTruthy();
    await user.click(undoButton());
    await screen.findByText("Saved");
  });

  test("a built-in is read-only: it has no Edit, and its outline edits go to a copy Save stores as new", async () => {
    const { b, user, box } = await open("copy");
    expect(b.summaries.find((s) => s.id === "w-ss")?.builtin).toBe(true);
    await user.click(row(/Warm-up/));
    await user.click(tool("Duplicate"));
    await waitFor(() => expect(textOf(box)).toBe(text("dup-warm-up")));
    await user.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByText("Saved");
    expect(calls(b, "workout_save").map((c) => c.args.id)).toEqual([null]);
    await user.click(screen.getByRole("button", { name: "Back to library" }));
    // The built-ins come first in the library, the rider's copy after them.
    await user.click(
      (await screen.findAllByRole("option", { name: /Sweet spot 3x12/ }))[0]!,
    );
    expect(screen.queryByRole("button", { name: "Edit" })).toBeNull();
  });
});

describe("round 2: focus, races, read-back, nesting (#653)", () => {
  test("deleting the only node leaves focus on Add step; Cmd/Ctrl+Z there brings it back, focused", async () => {
    install({ trees: {} });
    const user = userEvent.setup();
    render(
      <>
        <Rail view="workouts" />
        <Workouts />
      </>,
    );
    await user.click(
      await screen.findByRole("button", { name: "New workout" }),
    );
    const box = (await screen.findByRole("textbox", {
      name: "Text",
    })) as HTMLElement;
    await user.click(tool("Add step"));
    await waitFor(() => expect(textOf(box)).toBe(text("one-step")));
    await user.click(tool("Delete"));
    await waitFor(() => expect(textOf(box)).toBe(""));
    await screen.findByText("No steps yet: add one, or write one in Text.");
    expect(document.activeElement).toBe(tool("Add step"));
    await user.keyboard("{Control>}z{/Control}");
    await waitFor(() => expect(textOf(box)).toBe(text("one-step")));
    // The restored node is selected again (the step was left with it) and focused.
    await waitFor(() =>
      expect(document.activeElement).toBe(row(/5:00 · 50 %/)),
    );
    expect(row(/5:00 · 50 %/).getAttribute("aria-selected")).toBe("true");
  });

  test("text typed while the core checks an outline edit is kept: the edit is refused", async () => {
    const { user, box } = await open();
    const internals = (
      window as unknown as {
        __TAURI_INTERNALS__: {
          invoke: (c: string, a?: unknown, o?: unknown) => Promise<unknown>;
        };
      }
    ).__TAURI_INTERNALS__;
    const inner = internals.invoke;
    let release = () => {};
    const gate = new Promise<void>((r) => (release = r));
    internals.invoke = async (c, a, o) => {
      if (c === "print_dsl") await gate;
      return inner(c, a, o);
    };
    await user.click(row(/Cool-down/));
    await user.click(tool("Delete"));
    // The rider types while print_dsl is held: a draft the core refuses, so the saved tree does not change.
    setText(box, text("missing-target"));
    release();
    expect(
      (
        await screen.findByText(
          /The text was edited while this edit was checked/,
        )
      ).textContent,
    ).toContain("Nothing was changed.");
    expect(textOf(box)).toBe(text("missing-target"));
    expect((undoButton() as HTMLButtonElement).disabled).toBe(true);
  });

  test("the tree committed is the one the core read back (its fresh ids)", async () => {
    const { b, user, box } = await open();
    await user.click(row(/Warm-up/));
    await user.click(tool("Duplicate"));
    await waitFor(() => expect(textOf(box)).toBe(text("dup-warm-up")));
    await waitFor(() => {
      const last = calls(b, "compile_workout").at(-1)!.args.workout as {
        nodes: { id: string }[];
      };
      expect(
        last.nodes.map((n) => n.id).every((id) => id.startsWith("dsl-")),
      ).toBe(true);
    });
  });

  test("move and delete inside a repeat; deleting its last child removes the repeat", async () => {
    const { user, box } = await open();
    await user.click(row(/Rest/));
    await user.click(tool("Move up"));
    await waitFor(() => expect(textOf(box)).toBe(text("rest-up")));
    expect((document.activeElement as HTMLElement).textContent).toMatch(/Rest/);
    await user.click(undoButton());
    await waitFor(() => expect(textOf(box)).toBe(text("sweet-spot")));
    await user.click(row(/Sweet spot/));
    await user.click(tool("Delete"));
    await waitFor(() => expect(textOf(box)).toBe(text("no-sweet-spot")));
    expect((document.activeElement as HTMLElement).textContent).toMatch(/Rest/);
    // Rest is the repeat's only child now: Delete takes the repeat with it.
    await user.click(tool("Delete"));
    await waitFor(() => expect(textOf(box)).toBe(text("no-repeat")));
    expect((document.activeElement as HTMLElement).textContent).toMatch(
      /Cool-down/,
    );
    await user.click(undoButton());
    await waitFor(() => expect(textOf(box)).toBe(text("no-sweet-spot")));
    await user.click(undoButton());
    await waitFor(() => expect(textOf(box)).toBe(text("sweet-spot")));
  });

  test("durations are whole seconds: 0.1 min is 6 s; a negative part or a fraction of a second is refused", async () => {
    const { b, user, box } = await open();
    await user.click(row(/Warm-up/));
    const min = () => screen.getByRole("textbox", { name: "Minutes" });
    const sec = () => screen.getByRole("textbox", { name: "Seconds" });
    const apply = () =>
      user.click(screen.getByRole("button", { name: "Apply" }));
    await user.clear(min());
    await user.type(min(), "5");
    await user.clear(sec());
    await user.type(sec(), "-30");
    await apply();
    await screen.findByText(/Seconds: not below 0\. Nothing was changed\./);
    await user.clear(sec());
    await user.type(sec(), "1.5");
    await apply();
    await screen.findByText(
      /Duration: whole seconds only\. Nothing was changed\./,
    );
    expect(calls(b, "print_dsl")).toEqual([]);
    await user.clear(min());
    await user.type(min(), "0.1");
    await user.clear(sec());
    await user.type(sec(), "0");
    await apply();
    await waitFor(() => expect(textOf(box)).toBe(text("warm-up-6s")));
  });

  test("Cmd/Ctrl+Z in a form box undoes the workout and drops values not applied", async () => {
    const { user, box } = await open();
    await user.click(row(/^3 ×$/));
    const count = screen.getByRole("textbox", { name: "Count" });
    await user.clear(count);
    await user.type(count, "4{Enter}");
    await waitFor(() => expect(textOf(box)).toBe(text("sweet-spot-4x")));
    // Typed, not applied: Cmd/Ctrl+Z in the box undoes the count; the form shows the restored node's 3, not the 7.
    await user.clear(count);
    await user.type(count, "7");
    expect(document.activeElement).toBe(count);
    await user.keyboard("{Control>}z{/Control}");
    await waitFor(() => expect(textOf(box)).toBe(text("sweet-spot")));
    expect(
      (screen.getByRole("textbox", { name: "Count" }) as HTMLInputElement)
        .value,
    ).toBe("3");
  });
});

describe("nodeFrom", () => {
  const step: WorkoutNode = {
    type: "step",
    id: "s",
    label: "Ramp",
    end: { kind: "time", s: null },
    target: { kind: "ramp", from: null, to: 70 },
    cadence: null,
  };
  test("a box left as shown keeps the node's own value, a null one included", () => {
    const r = nodeFrom(step, {
      label: "Ramp up",
      min: "",
      sec: "",
      from: "",
      to: "70",
    });
    expect(r).toEqual({ ok: true, node: { ...step, label: "Ramp up" } });
  });
  test("a typed value replaces a null one", () => {
    const r = nodeFrom(step, {
      label: "Ramp",
      min: "",
      sec: "",
      from: "50",
      to: "70",
    });
    expect(r).toEqual({
      ok: true,
      node: { ...step, target: { kind: "ramp", from: 50, to: 70 } },
    });
  });
});

describe("round 3 (#653)", () => {
  test("Cmd/Ctrl+Z with nothing to undo leaves no stale refocus: a later text commit keeps focus in Text", async () => {
    const { user, box } = await open();
    await user.click(row(/Warm-up/));
    expect((undoButton() as HTMLButtonElement).disabled).toBe(true);
    await user.keyboard("{Control>}z{/Control}"); // focus on a row, nothing to undo
    await user.keyboard("{Control>}{Shift>}z{/Shift}{/Control}"); // nothing to redo
    box.focus();
    setText(box, text("sweet-spot-4x"));
    await waitFor(() => expect(row(/^4 ×$/)).toBeTruthy());
    expect(document.activeElement).toBe(box);
  });

  const rest: WorkoutNode = {
    type: "step",
    id: "rest",
    label: "Rest",
    end: { kind: "time", s: 240 },
    target: { kind: "pct_ftp", value: 55 },
    cadence: null,
  };
  const warm: WorkoutNode = { ...rest, id: "warm", label: "Warm-up" };
  const rep = (id: string, children: WorkoutNode[]): WorkoutNode => ({
    type: "repeat",
    id,
    label: null,
    count: 2,
    children,
    progression: null,
    skipLastRest: null,
  });
  const del = (nodes: WorkoutNode[], id: string) =>
    outlineActions(nodes, id).find((a) => a.label === "Delete")!.change!(
      () => "x",
    );

  test("deleting the only child of nested single-child repeats removes every repeat it would leave empty", () => {
    const nodes = [warm, rep("outer", [rep("inner", [rest])])];
    expect(soleChildOf(nodes, "rest")?.id).toBe("outer");
    expect(del(nodes, "rest")).toEqual({ nodes: [warm], select: "warm" });
    // An outer repeat with another child keeps it; only the emptied inner repeat goes.
    const other = { ...rest, id: "easy", label: "Easy" };
    const mixed = [warm, rep("outer", [rep("inner", [rest]), other])];
    expect(soleChildOf(mixed, "rest")?.id).toBe("inner");
    expect(del(mixed, "rest").nodes).toEqual([warm, rep("outer", [other])]);
    expect(soleChildOf(mixed, "easy")).toBeNull();
  });

  test("an empty duration box beside a filled one counts as 0", () => {
    const none: WorkoutNode = { ...rest, end: { kind: "time", s: null } };
    const at = (min: string, sec: string) => {
      const r = nodeFrom(none, { label: "Rest", min, sec, value: "55" });
      return r.ok && r.node.type === "step" && r.node.end.kind === "time"
        ? r.node.end.s
        : r;
    };
    expect(at("5", "")).toBe(300);
    expect(at("", "45")).toBe(45);
    expect(at("", "")).toBe(null); // left as shown: stays none
    expect(
      nodeFrom(rest, { label: "Rest", min: "", sec: "", value: "55" }),
    ).toEqual({
      ok: false,
      why: "Minutes: enter a number",
    });
  });
});
