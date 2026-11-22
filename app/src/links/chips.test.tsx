// @vitest-environment jsdom
// The link chips (ADR-0002 §5.1.4 "Failure UX", #681): the word of each keeper phase and ride link, and the chips on
// Today, Workouts and Settings → Devices in the real app shell, read from the snapshot channel's `devices` and following
// it frame by frame; Retry where it is offered, sent as `links_intent(Retry)` (§5.1.2) through the window's reporter.
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { App } from "../App";
import type {
  DeviceView,
  LinkIntent,
  LinkPhase,
  LinkStatus,
  PairedDevice,
} from "../bindings";
import { markSeen } from "../build";
import { navigate } from "../router";
import { devicesOf, installUi, type UiBackend } from "../stream/uiBackend";
import { installDevices } from "../views/devicesBackend";
import { install as installToday } from "../views/todayBackend";
import { install as installWorkouts } from "../views/workoutsBackend";
import { chipWord, offersRetry, type ChipWord } from "./chips";

const BUILD = "0.2.0-rc.1 · 7516045";
const readBuild = () => Promise.resolve(BUILD);
const RETRY: LinkIntent = { kind: "retry" };

const neo: PairedDevice = {
  role: "trainer",
  id: "neo-1",
  name: "Tacx Neo 55624",
  serial: "55624",
  model: "Neo",
};
const hrm: PairedDevice = {
  role: "heart_rate",
  id: "hrm-1",
  name: "HRM-Pro",
  serial: null,
  model: null,
};

type Invoke = (cmd: string, args?: unknown) => Promise<unknown>;
const invoke = () =>
  (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } })
    .__TAURI_INTERNALS__.invoke;
const intents = (calls: { cmd: string; args: Record<string, unknown> }[]) =>
  calls.filter((c) => c.cmd === "links_intent").map((c) => c.args.intent);

const keeper = (phase: LinkPhase): DeviceView =>
  devicesOf({ trainer: neo }, {}, { trainer: phase }).trainer;
const riding = (link: LinkStatus): DeviceView =>
  devicesOf({ trainer: neo }, { trainer: link }).trainer;

describe("chipWord", () => {
  const w = (glyph: string, word: string, state: ChipWord["state"]) => ({
    glyph,
    word,
    state,
  });
  test.each<[string, DeviceView, ReturnType<typeof w>]>([
    ["nothing paired", devicesOf({}).trainer, w("–", "not paired", "unpaired")],
    ["cold", keeper("cold"), w("–", "not connected", "cold")],
    ["searching", keeper("searching"), w("↻", "searching…", "searching")],
    ["connecting", keeper("connecting"), w("↻", "connecting…", "connecting")],
    // the trainer's handshake is the end of its connecting, not a word of its own
    ["handshake", keeper("handshake"), w("↻", "connecting…", "connecting")],
    ["warm", keeper("warm"), w("✓", "ready", "ready")],
    ["not found", keeper("not_found"), w("⚠", "not found", "not-found")],
    [
      "held by the ride, ready",
      riding("ready"),
      w("✓", "connected", "connected"),
    ],
    [
      "held by the ride, reconnecting",
      riding("connecting"),
      w("↻", "connecting…", "connecting"),
    ],
    ["held by the ride, lost", riding("lost"), w("⚠", "lost", "lost")],
  ])("%s", (_, device, word) => {
    expect(chipWord("trainer", device)).toMatchObject(word);
  });

  test("only a device that is not found has a hint: hedged for the trainer (#314), keeps looking for the strap", () => {
    expect(chipWord("trainer", keeper("not_found")).hint).toBe(
      "is another app using it?",
    );
    expect(chipWord("strap", keeper("not_found")).hint).toBe("keeps looking");
    for (const p of ["cold", "searching", "connecting", "warm"] as const)
      expect(chipWord("trainer", keeper(p)).hint).toBeNull();
    expect(chipWord("trainer", riding("lost")).hint).toBeNull();
  });
});

describe("a loaded ride holds the links", () => {
  // keeper/contract_tests.rs `each_device_has_one_source_an_unlinked_strap_stays_the_keepers`: the strap that missed
  // the load stays the keeper's, `not_found`, beside the ride's trainer.
  const held = devicesOf(
    { trainer: neo, strap: hrm },
    { trainer: "ready" },
    { strap: "not_found" },
  );
  test("a strap not found is out of the ride: no 'keeps looking', no Retry", () => {
    expect(chipWord("strap", held.strap, true)).toMatchObject({
      state: "not-found",
      hint: "the ride runs without it",
    });
    expect(offersRetry(held)).toBe(false);
    // the same strap with no ride: the keeper keeps looking, and Retry is offered
    const free = devicesOf(
      { trainer: neo, strap: hrm },
      {},
      { trainer: "warm", strap: "not_found" },
    );
    expect(chipWord("strap", free.strap)).toMatchObject({
      hint: "keeps looking",
    });
    expect(offersRetry(free)).toBe(true);
  });
});

const ready = async () => {
  const main = await screen.findByRole("main");
  await waitFor(() => expect(document.activeElement).toBe(main));
};
const chip = (role: "trainer" | "strap") =>
  document.querySelector<HTMLElement>(`[data-link-chip='${role}']`);

beforeEach(() => {
  localStorage.clear();
  markSeen(BUILD);
  navigate("today");
  vi.stubGlobal("matchMedia", (q: string) => ({
    matches: false,
    media: q,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
});
afterEach(() => {
  clearMocks();
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

/** Moves the keeper's phases, as its attempts do; the next frame carries them. */
function phases(ui: UiBackend, p: { trainer?: LinkPhase; strap?: LinkPhase }) {
  act(() => {
    ui.truth.devices = devicesOf({ trainer: neo, strap: hrm }, {}, p);
    ui.changed();
  });
}

describe("Today", () => {
  test("each device's chip follows the keeper's phases as it warms up", async () => {
    const ui = installUi({ devices: devicesOf({ trainer: neo, strap: hrm }) });
    installToday({ others: invoke() });
    render(<App readBuild={readBuild} />);
    await ready();
    await waitFor(() => expect(chip("trainer")?.dataset.state).toBe("cold"));
    const walk: [LinkPhase, LinkPhase, string, string][] = [
      ["searching", "searching", "↻ searching…", "↻ searching…"],
      ["handshake", "connecting", "↻ connecting…", "↻ connecting…"],
      ["warm", "warm", "✓ ready", "✓ ready"],
    ];
    for (const [t, s, tw, sw] of walk) {
      phases(ui, { trainer: t, strap: s });
      await waitFor(() => expect(chip("trainer")?.textContent).toBe(tw));
      expect(chip("strap")?.textContent).toBe(sw);
    }
    // Nothing is missing: no Retry, no hint.
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    expect(document.querySelector(".link-hint")).toBeNull();
  });

  test("a trainer that is not found: ⚠, the hedged hint, and Retry sends links_intent(Retry)", async () => {
    const ui = installUi({
      devices: devicesOf(
        { trainer: neo, strap: hrm },
        {},
        { trainer: "not_found", strap: "warm" },
      ),
    });
    installToday({ others: invoke() });
    render(<App readBuild={readBuild} />);
    await ready();
    await waitFor(() =>
      expect(chip("trainer")?.textContent).toBe("⚠ not found"),
    );
    expect(screen.getByText("Trainer: is another app using it?")).toBeTruthy();
    await waitFor(() =>
      expect(intents(ui.calls)).toEqual([{ kind: "likely", ready: false }]),
    );
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(intents(ui.calls).at(-1)).toEqual(RETRY));
    // Found on the attempt Retry started: the chip says so, and Retry goes.
    phases(ui, { trainer: "warm", strap: "warm" });
    await waitFor(() => expect(chip("trainer")?.textContent).toBe("✓ ready"));
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
  });

  test("a loaded ride's lost trainer link is the chip's loss", async () => {
    installUi({
      devices: devicesOf(
        { trainer: neo, strap: hrm },
        { trainer: "lost", strap: "ready" },
      ),
    });
    installToday({ others: invoke() });
    render(<App readBuild={readBuild} />);
    await ready();
    await waitFor(() => expect(chip("trainer")?.textContent).toBe("⚠ lost"));
    expect(chip("strap")?.textContent).toBe("✓ connected");
    // The ride holds the links: Retry would change nothing (the keeper is inactive), so none is offered.
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
  });
  test("with a ride loaded and the strap not in it, Today says so and offers no Retry", async () => {
    const ui = installUi({
      devices: devicesOf(
        { trainer: neo, strap: hrm },
        { trainer: "ready" },
        { strap: "not_found" },
      ),
    });
    installToday({ others: invoke() });
    render(<App readBuild={readBuild} />);
    await ready();
    await waitFor(() => expect(chip("strap")?.textContent).toBe("⚠ not found"));
    expect(chip("trainer")?.textContent).toBe("✓ connected");
    expect(screen.getByText("HR: the ride runs without it")).toBeTruthy();
    expect(screen.queryByText(/keeps looking/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    expect(intents(ui.calls)).not.toContainEqual(RETRY);
  });
});

describe("Workouts", () => {
  test("the chips under Ride this: phases, the strap's hint, and Retry", async () => {
    const ui = installUi({
      devices: devicesOf(
        { trainer: neo, strap: hrm },
        {},
        { trainer: "connecting", strap: "not_found" },
      ),
    });
    installWorkouts({ ui });
    navigate("workouts");
    render(<App readBuild={readBuild} />);
    await userEvent.click(
      await screen.findByRole("option", { name: /Sweet spot 3x12/ }),
    );
    const group = await screen.findByRole("group", { name: "Device links" });
    await waitFor(() =>
      expect(group.textContent).toContain("Trainer ↻ connecting…"),
    );
    expect(group.textContent).toContain("HR ⚠ not found");
    expect(group.textContent).toContain("HR: keeps looking");
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(intents(ui.calls).at(-1)).toEqual(RETRY));
  });
});

describe("Settings → Devices", () => {
  test("each paired row shows its chip, released for pairing, and never Retry", async () => {
    const { ui } = installDevices();
    act(() => {
      ui.truth.devices = devicesOf({ trainer: neo, strap: hrm });
      ui.changed();
    });
    navigate("settings", "devices");
    render(<App readBuild={readBuild} />);
    await screen.findByRole("heading", { name: "Devices" });
    await waitFor(() =>
      expect(chip("trainer")?.textContent).toBe("– not connected"),
    );
    expect(chip("strap")?.textContent).toBe("– not connected");
    // Even a device the keeper reports not found offers no Retry here: it would connect what this view released.
    phases(ui, { trainer: "not_found", strap: "cold" });
    await waitFor(() =>
      expect(chip("trainer")?.textContent).toBe("⚠ not found"),
    );
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    expect(intents(ui.calls)).not.toContainEqual(RETRY);
  });
});
