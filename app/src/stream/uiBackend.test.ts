// @vitest-environment jsdom
// The snapshot channel's backend models the link keeper's `links_intent` as the core answers it with no ride or a
// loaded one (ADR-0002 §5.1.3, #680): every refusal listed in its header, in the core's order, and the effect on the
// channel's `devices`. The ride backend's own model (ride/linksBackend.test.ts) covers the ride's side.
import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, test } from "vitest";
import { api } from "../api";
import { commands } from "../bindings";
import type { LinkIntent, PairedDevice } from "../bindings";
import { status } from "./testBackend";
import { SUPERSEDED, devicesOf, installUi, rideView } from "./uiBackend";

afterEach(() => clearMocks());

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
  name: "HRM-Dual",
  serial: null,
  model: null,
};
const WARM = devicesOf(
  { trainer: neo, strap: hrm },
  {},
  { trainer: "warm", strap: "warm" },
);
const send = (intent: LinkIntent) => api(commands.linksIntent(intent));
const OK = { status: "ok", data: null };
const refused = (code: string, message: string) => ({
  status: "error",
  error: { code, message },
});
const RELEASES = ["hidden", "sleep", "devices", "rider_release"] as const;

describe("links_intent with no ride", () => {
  test.each(RELEASES)(
    "Release(%s) lets every paired device go",
    async (reason) => {
      const ui = installUi({ devices: WARM });
      expect(await send({ kind: "release", reason })).toEqual(OK);
      expect(ui.truth.devices.trainer.phase).toBe("cold");
      expect(ui.truth.devices.strap.phase).toBe("cold");
      expect(ui.truth.devices.trainer.paired).toEqual(neo);
    },
  );

  test("Likely, Left and Background change no phase; nothing paired has none", async () => {
    const ui = installUi({ devices: WARM });
    for (const intent of [
      { kind: "likely", ready: true },
      { kind: "left" },
      { kind: "background" },
      { kind: "retry" },
    ] as const)
      expect(await send(intent)).toEqual(OK);
    expect(ui.truth.devices).toEqual(WARM);
    const bare = installUi();
    expect(await send({ kind: "release", reason: "hidden" })).toEqual(OK);
    expect(bare.truth.devices.trainer.phase).toBeNull();
  });

  test("the keeper cannot read the pairings: storage, before anything else", async () => {
    installUi({ devices: WARM, keeperStorage: true, quitting: true });
    expect(await send({ kind: "likely", ready: false })).toEqual(
      refused("storage", "database is locked"),
    );
  });

  test("after the quit Likely and Retry are refused, releases and the rest are not", async () => {
    installUi({ devices: WARM, quitting: true });
    const quitting = refused("conflict", "the application is quitting");
    expect(await send({ kind: "likely", ready: false })).toEqual(quitting);
    expect(await send({ kind: "retry" })).toEqual(quitting);
    expect(await send({ kind: "left" })).toEqual(OK);
    expect(await send({ kind: "background" })).toEqual(OK);
    expect(await send({ kind: "release", reason: "hidden" })).toEqual(OK);
  });

  test("a release recorded during a load and superseded by a rider action: conflict, once", async () => {
    const ui = installUi({ devices: WARM, loading: true, supersedes: true });
    expect(await send({ kind: "release", reason: "hidden" })).toEqual(
      refused("conflict", SUPERSEDED),
    );
    expect(await send({ kind: "release", reason: "hidden" })).toEqual(OK);
    // nothing of a load in flight is changed here
    expect(ui.truth.devices).toEqual(WARM);
  });

  test("a call without its intent is Tauri's plain-string rejection", async () => {
    installUi();
    const r = await api(
      commands.linksIntent(undefined as unknown as LinkIntent),
    );
    expect(r).toEqual(
      refused(
        "internal",
        "missing required key intent for command `links_intent`",
      ),
    );
  });
});

describe("links_intent with a ride", () => {
  const riding = devicesOf(
    { trainer: neo, strap: hrm },
    { trainer: "ready" },
    { strap: "not_found" },
  );

  test("a loaded, unstarted ride is discarded by Sleep and Release trainer, its devices cold", async () => {
    for (const reason of ["sleep", "rider_release"] as const) {
      const ui = installUi({ ride: rideView(), devices: riding });
      expect(await send({ kind: "release", reason })).toEqual(OK);
      expect(ui.truth.ride).toBeNull();
      expect(ui.truth.devices.trainer).toMatchObject({
        phase: "cold",
        link: null,
      });
      // the strap the ride did not hold keeps its phase
      expect(ui.truth.devices.strap.phase).toBe("not_found");
    }
  });

  test("a loaded ride keeps its links through Hidden, Devices, Left, Background and Likely", async () => {
    const ui = installUi({ ride: rideView(), devices: riding });
    for (const intent of [
      { kind: "release", reason: "hidden" },
      { kind: "release", reason: "devices" },
      { kind: "left" },
      { kind: "background" },
      { kind: "likely", ready: false },
    ] as const)
      expect(await send(intent)).toEqual(OK);
    expect(ui.truth.ride).not.toBeNull();
    expect(ui.truth.devices).toEqual(riding);
  });

  test("a started ride refuses Release trainer and keeps everything through the rest", async () => {
    const ui = installUi({
      ride: rideView({ status: status({ state: { kind: "riding" } }) }),
      devices: riding,
    });
    expect(await send({ kind: "release", reason: "rider_release" })).toEqual(
      refused("conflict", "the ride holds the trainer: end the ride first"),
    );
    expect(await send({ kind: "release", reason: "sleep" })).toEqual(OK);
    expect(ui.truth.ride).not.toBeNull();
    expect(ui.truth.devices).toEqual(riding);
  });

  test("the effect reaches the page as the channel's next frame", async () => {
    const ui = installUi({ devices: WARM });
    const frames: unknown[] = [];
    const id = await api(
      commands.uiOpen({ onmessage: (f: unknown) => frames.push(f) } as never),
    );
    expect(id.status).toBe("ok");
    await api(commands.uiAck(1, 0));
    await send({ kind: "release", reason: "devices" });
    expect(ui.frames.at(-1)?.devices.trainer.phase).toBe("cold");
  });
});
