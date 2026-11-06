// @vitest-environment jsdom
// The Ready sheet (DESIGN §5.4, #526): its state from today's data, and the ride screen before Start showing it.
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
import type { LinkChange } from "../bindings";
import { navigate } from "../router";
import { readyBanner, readyView, type ReadyState } from "./ReadySheet";
import { LINE_MAX } from "./banners";
import { Ride } from "./Ride";
import { installRide, type RideWorld } from "./rideBackend";
import { beginRide } from "./session";

const refused = { code: "storage", message: "cannot open the record" } as const;

describe("readyView", () => {
  const view = (over: Partial<Parameters<typeof readyView>[0]> = {}) =>
    readyView({
      status: true,
      links: {},
      hrLinked: true,
      refusal: null,
      ...over,
    });

  test("with no link reported the load's answer holds: the trainer linked, the strap as hrLinked says", () => {
    expect(view()).toEqual({
      state: "ready",
      trainer: "connected",
      hr: "connected",
      canStart: true,
    });
    expect(view({ hrLinked: false })).toMatchObject({
      state: "strap-missing",
      hr: "lost",
      canStart: true,
    });
  });

  test("before the status nothing is known and Start is not offered by the state", () => {
    expect(view({ status: false })).toMatchObject({
      state: "loading",
      trainer: "checking",
      hr: "checking",
    });
  });

  test("a trainer link that is not ready holds the start (§5.4)", () => {
    expect(view({ links: { trainer: "connecting" } })).toMatchObject({
      state: "connecting",
      canStart: false,
    });
    expect(view({ links: { trainer: "lost" } })).toMatchObject({
      state: "trainer-missing",
      trainer: "lost",
      canStart: false,
    });
  });

  test("the links outrank the load's answer", () => {
    expect(view({ hrLinked: false, links: { hr: "ready" } }).state).toBe(
      "ready",
    );
    expect(view({ links: { hr: "lost" } }).state).toBe("strap-missing");
  });

  test("most urgent first: a refusal, then loading, then the trainer, then the strap", () => {
    const all = { trainer: "lost", hr: "lost" } as const;
    expect(view({ refusal: refused, links: all }).state).toBe("error");
    expect(view({ status: false, links: all }).state).toBe("loading");
    expect(view({ links: all }).state).toBe("trainer-missing");
    expect(view({ links: { trainer: "connecting", hr: "lost" } }).state).toBe(
      "connecting",
    );
  });

  test("every banner line fits the band's 26 characters (§6.1)", () => {
    const states: ReadyState[] = [
      "error",
      "loading",
      "trainer-missing",
      "connecting",
      "strap-missing",
      "ready",
    ];
    for (const s of states) {
      const b = readyBanner(s);
      for (const l of [b.title, ...b.actions, ...b.status])
        expect(l.text.length, `${s}: ${l.text}`).toBeLessThanOrEqual(LINE_MAX);
      if (b.compact !== undefined)
        expect(b.compact.length).toBeLessThanOrEqual(LINE_MAX);
    }
  });
});

beforeEach(() => navigate("ride"));
afterEach(() => {
  cleanup();
  clearMocks();
});

type Rig = ReturnType<typeof installRide>;
async function open(
  over: Partial<RideWorld> = {},
  opts: {
    hrLinked?: boolean;
    workoutId?: string | null;
    before?: (rig: Rig) => void;
    settled?: boolean;
  } = {},
) {
  const rig = installRide(over);
  opts.before?.(rig);
  beginRide({
    rideId: "ride-1",
    workoutId: opts.workoutId === undefined ? "w1" : opts.workoutId,
    hrLinked: opts.hrLinked ?? true,
  });
  render(<Ride />);
  await waitFor(() => expect(rig.base.liveSubs()).toBe(3));
  if (opts.settled !== false)
    await waitFor(() => expect(rig.base.count("stream_snapshot")).toBe(1));
  return rig;
}
const links = (rig: Rig, l: LinkChange[]) =>
  act(async () => {
    rig.base.push({ links: l });
  });
const sheet = () => screen.getByRole("region", { name: "Ready check" });
const band = () =>
  document.querySelector("[data-region='banner']")?.textContent ?? "";
const start = () =>
  screen.getByRole("button", { name: "Start" }) as HTMLButtonElement;

describe("the ride screen before Start is the Ready sheet", () => {
  test("ready: both devices connected, the plan's first block, Start enabled and focused", async () => {
    await open();
    await waitFor(() => expect(sheet().dataset.ready).toBe("ready"));
    expect(band()).toContain("Ready to ride");
    const rows = within(sheet()).getAllByRole("listitem");
    expect(rows.map((r) => r.dataset.state)).toEqual([
      "connected",
      "connected",
    ]);
    await waitFor(() =>
      expect(screen.getByTestId("ready-plan").textContent).toBe(
        "ERG · starts with 3:00 warm-up",
      ),
    );
    // The traces come with the start: before it the column is the sheet's.
    expect(document.querySelector(".trace-stack")).toBeNull();
    expect(start().disabled).toBe(false);
    await waitFor(() => expect(document.activeElement).toBe(start()));
  });

  test("loading: until the stream answers nothing is the core's and Start waits", async () => {
    let release = () => {};
    const rig = await open(
      {},
      {
        settled: false,
        before: (r) => (release = r.base.hold("stream_snapshot")),
      },
    );
    expect(sheet().dataset.ready).toBe("loading");
    expect(band()).toContain("Loading the ride…");
    expect(within(sheet()).getAllByText(/checking…/)).toHaveLength(2);
    expect(start().disabled).toBe(true);
    // The metric column does not call a plan it has no status for "done".
    expect(screen.queryByText("Last block done")).toBeNull();
    expect(rig.count("ride_start")).toBe(0);
    await act(async () => release());
    await waitFor(() => expect(sheet().dataset.ready).toBe("ready"));
  });

  test("connecting: a trainer link coming up disables Start and says why", async () => {
    const rig = await open({ trainer: "connecting" });
    await links(rig, [{ device: "trainer", state: "connecting" }]);
    expect(sheet().dataset.ready).toBe("connecting");
    expect(band()).toContain("Connecting the NEO…");
    expect(sheet().textContent).toContain("Start waits until it is connected");
    expect(start().disabled).toBe(true);
    await links(rig, [{ device: "trainer", state: "ready" }]);
    rig.world.trainer = "ready";
    expect(sheet().dataset.ready).toBe("ready");
    expect(start().disabled).toBe(false);
  });

  test("trainer missing: a lost trainer link disables Start, with what to try", async () => {
    const rig = await open({ trainer: "connecting" });
    await links(rig, [{ device: "trainer", state: "lost" }]);
    expect(sheet().dataset.ready).toBe("trainer-missing");
    expect(band()).toContain("NEO not found");
    expect(band()).toContain("close apps using the NEO");
    expect(
      sheet().querySelector("[data-device='trainer']")?.textContent,
    ).toContain("⚠ not found");
    expect(start().disabled).toBe(true);
    await userEvent.click(start());
    expect(rig.count("ride_start")).toBe(0);
  });

  test("strap missing: the load linked no strap; Start stays enabled and the consequence is stated", async () => {
    const rig = await open({}, { hrLinked: false });
    await waitFor(() => expect(sheet().dataset.ready).toBe("strap-missing"));
    expect(band()).toContain("HR not found");
    expect(sheet().querySelector("[data-device='hr']")?.textContent).toContain(
      "this ride runs without heart rate",
    );
    expect(start().disabled).toBe(false);
    await userEvent.click(start());
    await waitFor(() => expect(rig.world.phase).toBe("active"));
  });

  test("error: a refused Start shows the core's reason in the sheet, and the next try clears it", async () => {
    const rig = await open({ recordFails: true });
    await userEvent.click(start());
    const alert = await within(sheet()).findByRole("alert");
    expect(alert.textContent).toBe(
      "The ride could not be recorded: cannot open the record",
    );
    expect(sheet().dataset.ready).toBe("error");
    expect(band()).toContain("Could not start");
    rig.world.recordFails = false;
    await userEvent.click(start());
    await waitFor(() => expect(rig.world.phase).toBe("active"));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  test("a free ride says so instead of a plan", async () => {
    await open({ blocks: [] }, { workoutId: null });
    expect(screen.getByTestId("ready-plan").textContent).toBe(
      "Free ride · no targets",
    );
  });

  test("the sheet leaves with the start: the traces and the ride's key row take its place", async () => {
    const rig = await open();
    await userEvent.click(start());
    await waitFor(() => expect(rig.world.phase).toBe("active"));
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "Ready check" })).toBeNull(),
    );
    expect(screen.getByRole("button", { name: "Pause" })).toBeTruthy();
    expect(band()).not.toContain("Ready to ride");
  });
});
