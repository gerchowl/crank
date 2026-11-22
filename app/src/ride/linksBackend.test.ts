// @vitest-environment jsdom
// The link commands of the ride test backend (`rideBackend.ts`) answer and refuse as the core does (ADR-0002 §5.1.3,
// §5.1.4): each refusal below is listed with its core file:line in the backend's header. `links_intent` answers `null`
// (#597): what an intent changes reaches the page only as the snapshot channel's next `devices` (ADR-0001 §4.3.6,
// §4.3.8). The link-ack flow (`links_open`, `links_ack`, their seq and coalescing rules) is no longer the page's; the
// facts the tests below assert are read off `rig.ui.truth.devices`, the state the window's `UiFrame` would carry.
import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, test } from "vitest";
import { api } from "../api";
import { commands } from "../bindings";
import type { DevicesView, LinkStatus, LinkView } from "../bindings";
import { COLD_LINKS, SUPERSEDED, installRide } from "./rideBackend";

afterEach(() => clearMocks());

const WARM: LinkView = {
  trainer: {
    phase: "warm",
    sample: { kind: "trainer", powerW: 180, cadenceRpm: 90 },
  },
  strap: { phase: "warm", sample: { kind: "heart_rate", bpm: 120 } },
};
const releaseTrainer = {
  kind: "release",
  reason: "rider_release",
} as const;
/** `links_intent`'s answer: it carries no state. */
const NOTHING = { status: "ok", data: null };

/** The link status of each role as the snapshot channel's `devices` carries it (ADR-0001 §4.3.6). */
const linksOf = (
  devices: DevicesView,
): { trainer: LinkStatus | null; strap: LinkStatus | null } => ({
  trainer: devices.trainer.link,
  strap: devices.strap.link,
});
/** The keeper's phase of each role, as `devices` carries it alongside the ride's link. */
const phasesOf = (devices: DevicesView) => ({
  trainer: devices.trainer.phase,
  strap: devices.strap.phase,
});

describe("links_intent", () => {
  test("a started ride refuses Release trainer and keeps its devices", async () => {
    const rig = installRide({ phase: "active", keeper: WARM });
    const refused = await api(commands.linksIntent(releaseTrainer));
    expect(refused).toEqual({
      status: "error",
      error: {
        code: "conflict",
        message: "the ride holds the trainer: end the ride first",
      },
    });
    expect(rig.world.phase).toBe("active");
    const sleep = await api(
      commands.linksIntent({ kind: "release", reason: "sleep" }),
    );
    expect(sleep).toEqual(NOTHING);
    expect(rig.world.phase).toBe("active");
    // The channel's `devices` shows the ride's trainer as riding throughout.
    expect(rig.ui.truth.devices.trainer.phase).toBe("riding");
  });

  test("Release trainer or Sleep unloads an unstarted ride; the next frame is cold", async () => {
    for (const reason of ["rider_release", "sleep"] as const) {
      const rig = installRide({ phase: "loaded", keeper: WARM });
      const done = await api(commands.linksIntent({ kind: "release", reason }));
      expect(done).toEqual(NOTHING);
      expect(rig.world.phase).toBe("none");
      // The ride's hold is released: each held device comes back cold on the channel.
      expect(phasesOf(rig.ui.truth.devices)).toEqual({
        trainer: "cold",
        strap: "cold",
      });
      clearMocks();
    }
  });

  test("every other intent leaves a loaded ride alone, its devices riding", async () => {
    const rig = installRide({ phase: "loaded", keeper: WARM });
    for (const intent of [
      { kind: "likely", ready: true },
      { kind: "retry" },
      { kind: "left" },
      { kind: "background" },
      { kind: "release", reason: "hidden" },
      { kind: "release", reason: "devices" },
    ] as const) {
      expect(await api(commands.linksIntent(intent))).toEqual(NOTHING);
    }
    expect(rig.world.phase).toBe("loaded");
    expect(phasesOf(rig.ui.truth.devices)).toEqual({
      trainer: "riding",
      strap: "riding",
    });
  });

  test("a release superseded by a rider action answers conflict and applies nothing", async () => {
    const rig = installRide({ phase: "none", keeper: WARM, supersedes: true });
    const r = await api(
      commands.linksIntent({ kind: "release", reason: "hidden" }),
    );
    expect(r).toEqual({
      status: "error",
      error: { code: "conflict", message: SUPERSEDED },
    });
    expect(rig.world.keeper).toEqual(WARM);
  });

  // A release is recorded, and can be superseded, only while the links are handed to a load in flight, `Ride::Handed`
  // (keeper/mod.rs `Keeper::intent` (~458)); with a ride loaded or started it is the ride's to answer.
  test("with a ride loaded or started a release is never superseded", async () => {
    let rig = installRide({ phase: "loaded", keeper: WARM, supersedes: true });
    const discarded = await api(
      commands.linksIntent({ kind: "release", reason: "sleep" }),
    );
    expect(discarded).toEqual(NOTHING);
    expect(rig.world.phase).toBe("none");
    expect(phasesOf(rig.ui.truth.devices)).toEqual({
      trainer: "cold",
      strap: "cold",
    });
    clearMocks();
    rig = installRide({ phase: "active", keeper: WARM, supersedes: true });
    const kept = await api(
      commands.linksIntent({ kind: "release", reason: "hidden" }),
    );
    expect(kept).toEqual(NOTHING);
    expect(rig.world.phase).toBe("active");
    expect(rig.ui.truth.devices.trainer.phase).toBe("riding");
  });

  test("after the quit Likely and Retry are refused; releases still answer", async () => {
    installRide({ phase: "none", quitting: true });
    for (const intent of [
      { kind: "likely", ready: false },
      { kind: "retry" },
    ] as const) {
      expect(await api(commands.linksIntent(intent))).toEqual({
        status: "error",
        error: { code: "conflict", message: "the application is quitting" },
      });
    }
    const r = await api(
      commands.linksIntent({ kind: "release", reason: "hidden" }),
    );
    expect(r).toEqual(NOTHING);
  });

  test("a store failure when the keeper starts is storage; a missing argument is Tauri's string", async () => {
    installRide({ phase: "none", keeperStorage: true });
    const r = await api(commands.linksIntent({ kind: "left" }));
    expect(r.status === "error" && r.error.code).toBe("storage");
    const bare = await api(
      commands.linksIntent(undefined as unknown as { kind: "left" }),
    );
    expect(bare.status === "error" && bare.error.code).toBe("internal");
  });

  test("an intent's effect is in the next `devices`, not its answer", async () => {
    const rig = installRide({ phase: "none", keeper: WARM });
    expect(phasesOf(rig.ui.truth.devices)).toEqual({
      trainer: "warm",
      strap: "warm",
    });
    const answer = await api(
      commands.linksIntent({ kind: "release", reason: "hidden" }),
    );
    expect(answer).toEqual(NOTHING);
    expect(phasesOf(rig.ui.truth.devices)).toEqual({
      trainer: "cold",
      strap: "cold",
    });
  });
});

// The core's frames in the same flows (crank-core keeper/contract_tests.rs, the
// `a_page_stalled_across_a_load_and_its_end_gets_cold_after_its_ack` and
// `each_device_has_one_source_an_unlinked_strap_stays_the_keepers` tests): `riding` while the ride holds a device,
// then `cold` with no sample, in the snapshot channel's next `devices`.
describe("the ride's hold on the links", () => {
  test("a ride load hands the devices over: the channel says riding", async () => {
    const rig = installRide({ phase: "none", keeper: WARM });
    rig.world.phase = "loaded";
    rig.publish();
    expect(linksOf(rig.ui.truth.devices)).toEqual({
      trainer: "ready",
      strap: "ready",
    });
    expect(rig.ui.truth.devices.trainer.phase).toBe("riding");
  });

  test("cancelling a loaded ride (End before Start) gives the devices back cold", async () => {
    const rig = installRide({ phase: "loaded", keeper: WARM });
    expect(rig.ui.truth.devices.trainer.phase).toBe("riding");
    expect(await api(commands.rideEnd())).toEqual({ status: "ok", data: null });
    expect(phasesOf(rig.ui.truth.devices)).toEqual({
      trainer: "cold",
      strap: "cold",
    });
  });

  test("ending a started ride gives the devices back cold", async () => {
    const rig = installRide({ phase: "active", keeper: WARM });
    await api(commands.rideEnd());
    expect(phasesOf(rig.ui.truth.devices)).toEqual({
      trainer: "cold",
      strap: "cold",
    });
    // The finished ride is reaped: a repeated End changes nothing.
    await api(commands.rideEnd());
    expect(phasesOf(rig.ui.truth.devices)).toEqual({
      trainer: "cold",
      strap: "cold",
    });
  });

  test("a strap that missed the load stays the keeper's: not_found beside the riding trainer", async () => {
    const keeper: LinkView = {
      trainer: WARM.trainer,
      strap: { phase: "not_found", sample: null },
    };
    const rig = installRide({ phase: "active", keeper, strapHanded: false });
    expect(phasesOf(rig.ui.truth.devices)).toEqual({
      trainer: "riding",
      strap: "not_found",
    });
    await api(commands.rideEnd());
    expect(phasesOf(rig.ui.truth.devices)).toEqual({
      trainer: "cold",
      strap: "not_found",
    });
  });

  // The core (keeper/contract_tests.rs `release_trainer_after_the_end_of_a_started_ride_is_accepted`): `ride_load`,
  // `ride_start`, `ride_end`, then `links_intent(RiderRelease)` answers `Ok` with both devices `cold`
  // (core.rs `Core::links_intent` (~1250) finds no session: core.rs `Core::end_ride` (~2225) waited for the reap).
  test("Release trainer after End is accepted: the ended ride holds nothing", async () => {
    const rig = installRide({ phase: "active", keeper: WARM });
    await api(commands.rideEnd());
    expect(await api(commands.linksIntent(releaseTrainer))).toEqual(NOTHING);
    expect(phasesOf(rig.ui.truth.devices)).toEqual({
      trainer: "cold",
      strap: "cold",
    });
  });

  // The core (keeper/contract_tests.rs `a_ride_that_finished_on_its_own_holds_its_devices_until_the_reap`): a ride
  // whose last block was skipped is `Finished` with its session still there; "Release trainer" is refused (the
  // session's state is not `Idle`) and its devices stay `riding` until the reaper takes it.
  test("a ride that finished by skipping its last block holds its devices until it is reaped", async () => {
    const rig = installRide({ phase: "active", keeper: WARM });
    while (rig.world.phase === "active") await api(commands.rideSkip());
    expect(rig.world.phase).toBe("finished");
    expect(rig.ui.truth.devices.trainer.phase).toBe("riding");
    const refused = await api(commands.linksIntent(releaseTrainer));
    expect(refused.status === "error" && refused.error.message).toBe(
      "the ride holds the trainer: end the ride first",
    );
    rig.reap();
    expect(phasesOf(rig.ui.truth.devices)).toEqual({
      trainer: "cold",
      strap: "cold",
    });
    expect(await api(commands.linksIntent(releaseTrainer))).toEqual(NOTHING);
  });

  test("End of a ride that finished on its own waits for its reap: the devices come back cold", async () => {
    const rig = installRide({ phase: "active", keeper: WARM });
    while (rig.world.phase === "active") await api(commands.rideSkip());
    expect(await api(commands.rideEnd())).toEqual({ status: "ok", data: null });
    expect(phasesOf(rig.ui.truth.devices)).toEqual({
      trainer: "cold",
      strap: "cold",
    });
    expect(await api(commands.linksIntent(releaseTrainer))).toEqual(NOTHING);
  });

  // The ride's end releases its hold whether or not the engine's status frame goes out (core.rs `Core::reap` drops the
  // hold with the session; the status is the engine's). The mock models the status frame short-circuit with
  // `publishes: false`: the next `UiFrame` is not sent, but the keeper is settled all the same, so the next heartbeat
  // or command carries cold devices. Observed on `rig.world.keeper`, the keeper's state the next frame would carry.
  test("the hold is released even when no status frame is published", async () => {
    const rig = installRide({
      phase: "active",
      keeper: WARM,
      publishes: false,
    });
    await api(commands.rideEnd());
    expect(rig.world.keeper.trainer?.phase).toBe("cold");
    expect(rig.world.keeper.strap?.phase).toBe("cold");
  });
});

// Note: the parts of the earlier `linksBackend.test.ts` that only tested link-channel flow control were dropped with
// `links_open` / `links_ack` themselves:
// - `describe("links_open / links_ack")` — the "one frame in flight: changes wait for the ack..." test (seq numbering
//   and the coalescing of state changes between acks) was link-channel-only; the ride-load hand-over it also checked
//   is covered above by "a ride load hands the devices over".
// - `describe("the ride's hold on the links")`:
//   - "a page stalled across a load and its end gets one cold frame after its ack" — link-ack coalescing only.
//   - "a page stalled across a ride whose strap missed the load gets one frame" — link-ack coalescing only.
// - `describe("an intent's effect arrives as a frame")`:
//   - "acked: the next frame carries it" — subsumed by the ui-level "an intent's effect is in the next `devices`" test.
//   - "unacked: it waits for the ack, then arrives as one frame of the latest state" — link-ack flow only.
// The verified use of `COLD_LINKS` to assert `cold` keeper phase is now expressed through the `devices` view (the ui
// channel's `devices` carries the keeper phase of each unheld role), so the `LinkFrame` import is gone.
// `COLD_LINKS` is kept as an import for other tests that still consume it, if any.
void COLD_LINKS;
