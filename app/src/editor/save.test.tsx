// @vitest-environment jsdom
// The library's save loop (#110, #643, DESIGN §5.7): New workout → edit → Save → listed → Ride this rides that id; Save
// is refused while the workout has an error; unsaved changes are asked about before leaving (Back or the rail) and
// before riding; a built-in is read-only and Duplicate & edit makes the rider's own copy; Delete asks first. On the
// workouts backend, whose `workout_save` refuses as the core does (workoutsBackend.ts).
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, test } from "vitest";
import { navigate, useRoute } from "../router";
import { Rail } from "../shell/Rail";
import { UiClient, type Timers } from "../stream/ui";
import { installUi } from "../stream/uiBackend";
import { UiProvider } from "../stream/uiHooks";
import { DRAFTS } from "../views/editorFixture";
import { resetRideLoad } from "../views/rideLoad";
import { Workouts } from "../views/Workouts";
import {
  install as installBackend,
  type Backend,
} from "../views/workoutsBackend";
import { GROUP_MS } from "./model";
import { typingClock } from "./WorkoutEditor";
import { windowClose, type CloseRequest } from "./windowClose";

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
  closing = null;
  destroyed = 0;
  windowClose.listen = (h) => {
    closing = h;
    return Promise.resolve(() => {
      if (closing === h) closing = null;
    });
  };
  windowClose.destroy = () => {
    destroyed++;
    return Promise.resolve();
  };
});
/** The window's close-request listener while the editor holds one (only with unsaved changes), and its closes. */
let closing: ((e: CloseRequest) => void) | null = null;
let destroyed = 0;

function Routed() {
  const route = useRoute();
  return (
    <>
      <Rail view={route.view} />
      <p data-testid="route">{route.view}</p>
      {route.view === "workouts" && <Workouts />}
    </>
  );
}
const neverClock: Timers = { setTimeout: () => 0, clearTimeout: () => {} };
/** Workouts on the backend: "Sweet spot 3x12" (built in) and "Easy spin" (the rider's) are the core's `sweet-spot`. */
function open(over: Partial<Backend> = {}) {
  const b = installBackend({
    ui: installUi(),
    trees: { "w-ss": "sweet-spot", "w-easy": "sweet-spot" },
    ...over,
  });
  render(
    <UiProvider client={new UiClient(neverClock)}>
      <Routed />
    </UiProvider>,
  );
  return { b, user: userEvent.setup() };
}
const text = (name: string) => DRAFTS[name]!.text;
const box = () =>
  screen.getByRole("textbox", { name: "Text" }) as HTMLTextAreaElement;
const setText = (t: string) =>
  fireEvent.change(box(), { target: { value: t } });
const button = (name: string) => screen.getByRole("button", { name });
const calls = (b: Backend, cmd: string) =>
  b.calls.filter((c) => c.cmd === cmd).map((c) => c.args);
const library = async () =>
  within(await screen.findByRole("listbox", { name: "Library" }))
    .getAllByRole("option")
    .map((o) => o.querySelector(".workouts-name")?.textContent);
const dialog = () => screen.getByRole("dialog");

async function edit(user: ReturnType<typeof userEvent.setup>, name: RegExp) {
  await user.click(await screen.findByRole("option", { name }));
  await user.click(await screen.findByRole("button", { name: "Edit" }));
  await screen.findByRole("textbox", { name: "Text" });
}

test("New → edit → Save → listed → Ride this loads that id", async () => {
  const { b, user } = open();
  await user.click(await screen.findByRole("button", { name: "New workout" }));
  // An empty workout has an error: it cannot be saved yet, and the reason is shown.
  await screen.findByText(
    /Can’t save until the error is fixed: Workout is empty/,
  );
  expect(screen.getByText("Not saved")).toBeTruthy();
  expect((button("Save") as HTMLButtonElement).disabled).toBe(true);
  expect((button("Ride this") as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByText("Save the workout to ride it.")).toBeTruthy();

  setText(text("sweet-spot"));
  await waitFor(() =>
    expect((button("Save") as HTMLButtonElement).disabled).toBe(false),
  );
  await user.clear(screen.getByRole("textbox", { name: "Name" }));
  await user.type(screen.getByRole("textbox", { name: "Name" }), "Threshold");
  await user.click(button("Save"));
  await screen.findByText("Saved");
  expect(calls(b, "workout_save")).toEqual([
    {
      id: null,
      draft: {
        name: "Threshold",
        ftp: 250,
        nodes: expect.any(Array) as unknown,
      },
    },
  ]);
  // Saved and unchanged: Save has nothing to store, Ride this is open.
  expect((button("Save") as HTMLButtonElement).disabled).toBe(true);

  await user.click(button("Back to library"));
  expect(await library()).toEqual([
    "Sweet spot 3x12",
    "Easy spin",
    "Threshold",
  ]);
  // Back picks the saved workout.
  expect((await screen.findByRole("heading", { level: 2 })).textContent).toBe(
    "Threshold",
  );
  await user.click(await screen.findByRole("button", { name: "Ride this" }));
  await waitFor(() =>
    expect(screen.getByTestId("route").textContent).toBe("ride"),
  );
  expect(calls(b, "ride_load")).toEqual([{ workoutId: "workout-1" }]);
});

test("Ride this in the editor rides the saved workout; with unsaved changes it asks first", async () => {
  const { b, user } = open();
  await edit(user, /Easy spin/);
  expect(screen.getByText("Saved")).toBeTruthy();
  setText(text("sweet-spot-4x"));
  await screen.findByText("Unsaved changes");
  await user.click(button("Ride this"));
  expect(within(dialog()).getByRole("heading").textContent).toBe(
    "Save changes to Easy spin before riding?",
  );
  // The safe answer has focus; Cancel keeps editing and loads nothing.
  expect(document.activeElement).toBe(
    within(dialog()).getByRole("button", { name: "Save" }),
  );
  await user.click(within(dialog()).getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(calls(b, "ride_load")).toEqual([]);

  await user.click(button("Ride this"));
  await user.click(within(dialog()).getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(screen.getByTestId("route").textContent).toBe("ride"),
  );
  expect(calls(b, "workout_save").map((a) => a.id)).toEqual(["w-easy"]);
  expect(calls(b, "ride_load")).toEqual([{ workoutId: "w-easy" }]);
  const four = DRAFTS["sweet-spot-4x"]!;
  expect(b.stored["w-easy"]?.dsl).toBe("printed" in four ? four.printed : null);
});

test("Discard before riding rides the workout as last saved and stores nothing", async () => {
  const { b, user } = open();
  await edit(user, /Easy spin/);
  setText(text("sweet-spot-4x"));
  await screen.findByText("Unsaved changes");
  await user.click(button("Ride this"));
  await user.click(within(dialog()).getByRole("button", { name: "Discard" }));
  await waitFor(() =>
    expect(screen.getByTestId("route").textContent).toBe("ride"),
  );
  expect(calls(b, "workout_save")).toEqual([]);
  expect(calls(b, "ride_load")).toEqual([{ workoutId: "w-easy" }]);
});

test("leaving with unsaved changes asks: Cancel stays, Discard drops them, Save stores them", async () => {
  const { b, user } = open();
  await edit(user, /Easy spin/);
  // No changes: Back leaves at once.
  await user.click(button("Back to library"));
  expect(screen.queryByRole("dialog")).toBeNull();
  await screen.findByRole("listbox", { name: "Library" });

  await user.click(await screen.findByRole("button", { name: "Edit" }));
  setText(text("sweet-spot-4x"));
  await screen.findByText("Unsaved changes");
  await user.click(button("Back to library"));
  expect(within(dialog()).getByRole("heading").textContent).toBe(
    "Save changes to Easy spin?",
  );
  // Escape is Cancel.
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(box().value).toBe(text("sweet-spot-4x"));

  await user.click(button("Back to library"));
  await user.click(within(dialog()).getByRole("button", { name: "Discard" }));
  await screen.findByRole("listbox", { name: "Library" });
  expect(calls(b, "workout_save")).toEqual([]);

  // Undoing back to the saved version is clean again: nothing to ask.
  await user.click(await screen.findByRole("button", { name: "Edit" }));
  setText(text("sweet-spot-4x"));
  await screen.findByText("Unsaved changes");
  await user.click(button("Undo"));
  await screen.findByText("Saved");

  setText(text("sweet-spot-4x"));
  await user.click(button("Back to library"));
  await user.click(within(dialog()).getByRole("button", { name: "Save" }));
  await screen.findByRole("listbox", { name: "Library" });
  expect(calls(b, "workout_save").map((a) => a.id)).toEqual(["w-easy"]);
});

test("the rail asks too, and goes where the rider asked once they answer", async () => {
  const { user } = open();
  await edit(user, /Easy spin/);
  setText(text("sweet-spot-4x"));
  await screen.findByText("Unsaved changes");
  await user.click(screen.getByRole("button", { name: /History/ }));
  expect(screen.getByTestId("route").textContent).toBe("workouts");
  await user.click(within(dialog()).getByRole("button", { name: "Cancel" }));
  expect(screen.getByTestId("route").textContent).toBe("workouts");
  await user.click(screen.getByRole("button", { name: /History/ }));
  await user.click(within(dialog()).getByRole("button", { name: "Discard" }));
  expect(screen.getByTestId("route").textContent).toBe("history");
});

test("the rail's Workouts closes a clean editor back to the library", async () => {
  const { user } = open();
  await edit(user, /Easy spin/);
  await user.click(screen.getByRole("button", { name: /Workouts/ }));
  await screen.findByRole("listbox", { name: "Library" });
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("Save is refused while the workout has an error or no name, and the core's refusal is shown", async () => {
  const { b, user } = open();
  await edit(user, /Easy spin/);
  const name = screen.getByRole("textbox", { name: "Name" });
  await user.clear(name);
  expect(screen.getByText("Give the workout a name to save it.")).toBeTruthy();
  expect((button("Save") as HTMLButtonElement).disabled).toBe(true);
  await user.type(name, "Easy spin");
  // An empty workout is an error-severity issue: Save waits for it to be fixed.
  setText(text("empty"));
  await screen.findByText(
    "Can’t save until the error is fixed: Workout is empty",
  );
  expect((button("Save") as HTMLButtonElement).disabled).toBe(true);
  expect((button("Save as new") as HTMLButtonElement).disabled).toBe(true);
  // A tree the core cannot compile at all cannot be saved.
  setText(text("too-long"));
  await screen.findByText("A workout that does not compile cannot be saved.");
  expect((button("Save") as HTMLButtonElement).disabled).toBe(true);
  // A text that does not parse cannot be saved either.
  setText(text("missing-target"));
  await screen.findByText("Fix the text to save the workout.");
  expect(calls(b, "workout_save")).toEqual([]);

  // The core's own refusal (here the store failing) is shown, and nothing is marked saved.
  setText(text("sweet-spot-4x"));
  b.fail.workout_save = { code: "storage", message: "database is locked" };
  await waitFor(() =>
    expect((button("Save") as HTMLButtonElement).disabled).toBe(false),
  );
  await user.click(button("Save"));
  expect((await screen.findByRole("alert")).textContent).toContain(
    "database is locked",
  );
  expect(screen.getByText("Unsaved changes")).toBeTruthy();
});

test("a built-in is read-only: no Edit or Delete, and Duplicate & edit saves the rider's own copy", async () => {
  const { b, user } = open();
  await user.click(await screen.findByRole("option", { name: /Sweet spot/ }));
  await screen.findByText("Built-in, read-only");
  expect(screen.queryByRole("button", { name: "Edit" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
  await user.click(button("Duplicate & edit"));
  expect(
    (screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value,
  ).toBe("Sweet spot 3x12 (copy)");
  expect(screen.getByText("Not saved")).toBeTruthy();
  // An unchanged copy can be saved as it is, as a new workout.
  await user.click(button("Save"));
  await screen.findByText("Saved");
  expect(calls(b, "workout_save").map((a) => a.id)).toEqual([null]);
  // Saved: now it is the rider's, so Save as new makes a second one.
  await user.click(button("Save as new"));
  await waitFor(() => expect(calls(b, "workout_save")).toHaveLength(2));
  await user.click(button("Back to library"));
  expect(await library()).toEqual([
    "Sweet spot 3x12",
    "Easy spin",
    "Sweet spot 3x12 (copy)",
    "Sweet spot 3x12 (copy)",
  ]);
  expect(b.summaries.find((w) => w.id === "w-ss")?.name).toBe(
    "Sweet spot 3x12",
  );
});

test("Delete asks, Cancel keeps it, Delete removes it from the library", async () => {
  const { b, user } = open();
  await user.click(await screen.findByRole("option", { name: /Easy spin/ }));
  await user.click(await screen.findByRole("button", { name: "Delete" }));
  expect(within(dialog()).getByRole("heading").textContent).toBe(
    "Delete Easy spin?",
  );
  // The safe answer has focus.
  expect(document.activeElement).toBe(
    within(dialog()).getByRole("button", { name: "Cancel" }),
  );
  await user.click(within(dialog()).getByRole("button", { name: "Cancel" }));
  expect(calls(b, "workout_delete")).toEqual([]);
  await user.click(button("Delete"));
  await user.click(within(dialog()).getByRole("button", { name: "Delete" }));
  await waitFor(async () =>
    expect(await library()).toEqual(["Sweet spot 3x12"]),
  );
  expect(calls(b, "workout_delete")).toEqual([{ id: "w-easy" }]);
});

test("the mock's library commands refuse what the core refuses", async () => {
  installBackend({ trees: { "w-easy": "sweet-spot" } });
  const nodes = DRAFTS["sweet-spot"]!;
  if (!("nodes" in nodes)) throw new Error("sweet-spot parses");
  const draft = (name: string) => ({ name, ftp: 250, nodes: nodes.nodes });
  const refused = (cmd: string, args: Record<string, unknown>) =>
    invoke(cmd, args).then(
      () => null,
      (e: unknown) => e,
    );
  expect(await invoke("workout_new")).toEqual({
    name: "New workout",
    ftp: 250,
    nodes: [],
  });
  expect(
    await refused("workout_save", { id: null, draft: draft("  ") }),
  ).toEqual({ code: "invalid_input", message: "a workout needs a name" });
  expect(
    await refused("workout_save", { id: null, draft: draft("x".repeat(121)) }),
  ).toMatchObject({ code: "invalid_input" });
  expect(
    await refused("workout_save", { id: "w-ss", draft: draft("Mine") }),
  ).toMatchObject({ code: "invalid_input" });
  expect(
    await refused("workout_save", { id: "w-gone", draft: draft("Mine") }),
  ).toEqual({ code: "not_found", message: 'no workout "w-gone"' });
  const empty = { name: "Empty", ftp: 250, nodes: [] };
  expect(await refused("workout_save", { id: null, draft: empty })).toEqual({
    code: "invalid_input",
    message: "Workout is empty",
  });
  expect(await refused("workout_delete", { id: "w-ss" })).toMatchObject({
    code: "invalid_input",
  });
  expect(await refused("workout_delete", { id: "w-gone" })).toEqual({
    code: "not_found",
    message: 'no workout "w-gone"',
  });
  // The name is trimmed, as the core trims it.
  const id = await invoke("workout_save", { id: null, draft: draft(" Mine ") });
  expect(await invoke("workouts_list")).toContainEqual(
    expect.objectContaining({ id, name: "Mine", builtin: false }),
  );
});

test("with Save disabled the dialog focuses Cancel, so one Enter discards nothing", async () => {
  const { b, user } = open();
  await edit(user, /Easy spin/);
  setText(text("missing-target"));
  await screen.findByText("Fix the text to save the workout.");
  await user.click(button("Back to library"));
  const save = within(dialog()).getByRole("button", { name: "Save" });
  expect((save as HTMLButtonElement).disabled).toBe(true);
  expect(document.activeElement).toBe(
    within(dialog()).getByRole("button", { name: "Cancel" }),
  );
  await user.keyboard("{Enter}");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(box().value).toBe(text("missing-target"));
  expect(calls(b, "workout_save")).toEqual([]);
});

test("Save waits for the text being read, so it stores the last keystrokes, also from the dialog", async () => {
  const { b, user } = open();
  await edit(user, /Easy spin/);
  // Hold the core's answer to the next parse: the text runs ahead of the saved tree meanwhile.
  const inner = (
    window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (c: string, a?: unknown, o?: unknown) => Promise<unknown>;
      };
    }
  ).__TAURI_INTERNALS__.invoke;
  let release = () => {};
  const held = new Promise<void>((r) => (release = r));
  mockIPC((cmd, payload) =>
    cmd === "parse_dsl"
      ? held.then(() => inner(cmd, payload))
      : inner(cmd, payload),
  );
  setText(text("sweet-spot-4x"));
  expect(screen.getByText("Reading the text…")).toBeTruthy();
  expect((button("Save") as HTMLButtonElement).disabled).toBe(true);
  await user.click(button("Back to library"));
  expect(
    (
      within(dialog()).getByRole("button", {
        name: "Save",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
  release();
  const save = within(dialog()).getByRole("button", { name: "Save" });
  await waitFor(() => expect((save as HTMLButtonElement).disabled).toBe(false));
  await user.click(save);
  await screen.findByRole("listbox", { name: "Library" });
  const four = DRAFTS["sweet-spot-4x"]!;
  expect(b.stored["w-easy"]?.dsl).toBe("printed" in four ? four.printed : null);
});

test("closing the window with unsaved changes asks first; Discard then closes it", async () => {
  const { user } = open();
  await edit(user, /Easy spin/);
  // No unsaved changes: nothing holds the close request, the window closes as always.
  expect(closing).toBeNull();
  setText(text("sweet-spot-4x"));
  await screen.findByText("Unsaved changes");
  await waitFor(() => expect(closing).not.toBeNull());
  let prevented = false;
  closing!({ preventDefault: () => (prevented = true) });
  expect(prevented).toBe(true);
  await screen.findByRole("dialog");
  // Cancel keeps the window and the edits.
  await user.click(within(dialog()).getByRole("button", { name: "Cancel" }));
  expect(destroyed).toBe(0);
  closing!({ preventDefault: () => {} });
  await user.click(
    await within(await screen.findByRole("dialog")).findByRole("button", {
      name: "Discard",
    }),
  );
  expect(destroyed).toBe(1);
});
