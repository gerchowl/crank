// @vitest-environment jsdom
// The link commands of the ride test backend (`rideBackend.ts`) answer and refuse as the core does (ADR-0002 §5.1.3,
// §5.1.4): each refusal below is listed with its core file:line in the backend's header. `links_intent` answers `null`
// (#597): what an intent changes reaches the page only as the link channel's next frame.
import { Channel } from "@tauri-apps/api/core";
import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, test } from "vitest";
import { api } from "../api";
import { commands } from "../bindings";
import type { LinkFrame, LinkView } from "../bindings";
import { COLD_LINKS, SUPERSEDED, installRide } from "./rideBackend";

afterEach(() => clearMocks());

const WARM: LinkView = {
  trainer: {
    phase: "warm",
    sample: { kind: "trainer", powerW: 180, cadenceRpm: 90 },
  },
  strap: { phase: "warm", sample: { kind: "heart_rate", bpm: 120 } },
};
const SEARCHING: LinkView = {
  trainer: { phase: "searching", sample: null },
  strap: { phase: "searching", sample: null },
};
const releaseTrainer = {
  kind: "release",
  reason: "rider_release",
} as const;
/** `links_intent`'s answer: it carries no state. */
const NOTHING = { status: "ok", data: null };

/** The page's link channel: every frame it received, and an ack of the last one. */
async function open() {
  const frames: LinkFrame[] = [];
  const channel = new Channel<LinkFrame>();
  channel.onmessage = (f) => frames.push(f);
  const opened = await api(commands.linksOpen(channel));
  if (opened.status !== "ok") throw new Error("open failed");
  const ack = () => api(commands.linksAck(opened.data, frames.at(-1)!.seq));
  return { frames, ack, id: opened.data };
}

describe("links_intent", () => {
  test("a started ride refuses Release trainer and keeps its devices", async () => {
    const rig = installRide({ phase: "active", keeper: WARM });
    const { frames, ack } = await open();
    await ack();
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
    expect(frames.map((f) => f.links.trainer?.phase)).toEqual(["riding"]);
  });

  test("Release trainer or Sleep unloads an unstarted ride; the next frame is cold", async () => {
    for (const reason of ["rider_release", "sleep"] as const) {
      const rig = installRide({ phase: "loaded", keeper: WARM });
      const { frames, ack } = await open();
      await ack();
      const done = await api(commands.linksIntent({ kind: "release", reason }));
      expect(done).toEqual(NOTHING);
      expect(frames.at(-1)?.links).toEqual(COLD_LINKS);
      expect(rig.world.phase).toBe("none");
      clearMocks();
    }
  });

  test("every other intent leaves a loaded ride alone, its devices riding", async () => {
    const rig = installRide({ phase: "loaded", keeper: WARM });
    const { frames, ack } = await open();
    await ack();
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
    expect(frames).toHaveLength(1);
    expect(frames[0].links.strap?.phase).toBe("riding");
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
    expect((await open()).frames.at(-1)?.links).toEqual(COLD_LINKS);
    clearMocks();
    rig = installRide({ phase: "active", keeper: WARM, supersedes: true });
    const kept = await api(
      commands.linksIntent({ kind: "release", reason: "hidden" }),
    );
    expect(kept).toEqual(NOTHING);
    expect(rig.world.phase).toBe("active");
    expect((await open()).frames.at(-1)?.links.trainer?.phase).toBe("riding");
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
});

describe("links_open / links_ack", () => {
  test("one frame in flight: changes wait for the ack and arrive as one frame of the latest state", async () => {
    const rig = installRide({ phase: "none" });
    const frames: LinkFrame[] = [];
    const channel = new Channel<LinkFrame>();
    channel.onmessage = (f) => frames.push(f);
    const opened = await api(commands.linksOpen(channel));
    if (opened.status !== "ok") throw new Error("open failed");
    expect(frames).toEqual([{ seq: 1, links: COLD_LINKS }]);
    rig.setKeeper({
      ...COLD_LINKS,
      trainer: { phase: "searching", sample: null },
    });
    rig.setKeeper(WARM);
    expect(frames).toHaveLength(1);
    // an ack of another frame opens nothing
    await api(commands.linksAck(opened.data, 7));
    expect(frames).toHaveLength(1);
    await api(commands.linksAck(opened.data, 1));
    // the seq is the state's number: the coalesced `searching` state took 2
    expect(frames).toEqual([
      { seq: 1, links: COLD_LINKS },
      { seq: 3, links: WARM },
    ]);
    const stale = await api(commands.linksAck(opened.data + 1, 3));
    expect(stale).toEqual({
      status: "error",
      error: { code: "not_found", message: "unknown link channel" },
    });
  });

  test("a ride load hands the devices over: the channel says riding", async () => {
    const rig = installRide({ phase: "none", keeper: WARM });
    const frames: LinkFrame[] = [];
    const channel = new Channel<LinkFrame>();
    channel.onmessage = (f) => frames.push(f);
    const opened = await api(commands.linksOpen(channel));
    if (opened.status !== "ok") throw new Error("open failed");
    await api(commands.linksAck(opened.data, 1));
    rig.world.phase = "loaded";
    rig.publish();
    expect(frames.at(-1)?.links.trainer?.phase).toBe("riding");
  });
});

// The core's frames in the same flows (crank-core keeper/contract_tests.rs, the
// `a_page_stalled_across_a_load_and_its_end_gets_cold_after_its_ack` and
// `each_device_has_one_source_an_unlinked_strap_stays_the_keepers` tests): `riding` while the ride holds a device,
// then `cold` with no sample, in the next frame.
describe("the ride's hold on the links", () => {
  const seqs = (frames: LinkFrame[]) => frames.map((f) => f.seq);

  test("cancelling a loaded ride (End before Start) gives the devices back cold", async () => {
    installRide({ phase: "loaded", keeper: WARM });
    const { frames, ack } = await open();
    expect(frames.at(-1)?.links.trainer?.phase).toBe("riding");
    await ack();
    expect(await api(commands.rideEnd())).toEqual({
      status: "ok",
      data: { rideId: null, forced: false },
    });
    expect(frames.at(-1)?.links).toEqual(COLD_LINKS);
    expect(seqs(frames)).toEqual([1, 2]);
  });

  test("ending a started ride gives the devices back cold", async () => {
    installRide({ phase: "active", keeper: WARM });
    const { frames, ack } = await open();
    await ack();
    await api(commands.rideEnd());
    expect(frames.at(-1)?.links).toEqual(COLD_LINKS);
    // the finished ride is reaped: a repeated End changes nothing
    await ack();
    await api(commands.rideEnd());
    expect(frames).toHaveLength(2);
  });

  test("a page stalled across a load and its end gets one cold frame after its ack", async () => {
    const rig = installRide({ phase: "none", keeper: WARM });
    const { frames, ack } = await open();
    expect(frames).toEqual([{ seq: 1, links: WARM }]);
    rig.world.phase = "loaded";
    rig.publish();
    rig.world.phase = "active";
    rig.publish();
    await api(commands.rideEnd());
    expect(frames).toHaveLength(1);
    await ack();
    // `riding` (seq 2) was coalesced away
    expect(frames).toEqual([
      { seq: 1, links: WARM },
      { seq: 3, links: COLD_LINKS },
    ]);
  });

  test("a strap that missed the load stays the keeper's: not_found beside the riding trainer", async () => {
    const keeper: LinkView = {
      trainer: WARM.trainer,
      strap: { phase: "not_found", sample: null },
    };
    installRide({ phase: "active", keeper, strapHanded: false });
    const { frames, ack } = await open();
    expect(frames.at(-1)?.links).toEqual({
      trainer: { phase: "riding", sample: null },
      strap: { phase: "not_found", sample: null },
    });
    await ack();
    await api(commands.rideEnd());
    expect(frames.at(-1)?.links).toEqual({
      trainer: { phase: "cold", sample: null },
      strap: { phase: "not_found", sample: null },
    });
  });

  // The core (keeper/contract_tests.rs `release_trainer_after_the_end_of_a_started_ride_is_accepted`): `ride_load`,
  // `ride_start`, `ride_end`, then `links_intent(RiderRelease)` answers `Ok` with both devices `cold`
  // (core.rs `Core::links_intent` (~1250) finds no session: core.rs `Core::end_ride` (~2225) waited for the reap).
  test("Release trainer after End is accepted: the ended ride holds nothing", async () => {
    installRide({ phase: "active", keeper: WARM });
    const { frames, ack } = await open();
    await ack();
    await api(commands.rideEnd());
    expect(await api(commands.linksIntent(releaseTrainer))).toEqual(NOTHING);
    expect(frames.at(-1)?.links).toEqual(COLD_LINKS);
  });

  // The core (keeper/contract_tests.rs `a_ride_that_finished_on_its_own_holds_its_devices_until_the_reap`): a ride
  // whose last block was skipped is `Finished` with its session still there; "Release trainer" is refused (the
  // session's state is not `Idle`) and its devices stay `riding` until the reaper takes it.
  test("a ride that finished by skipping its last block holds its devices until it is reaped", async () => {
    const rig = installRide({ phase: "active", keeper: WARM });
    const { frames, ack } = await open();
    await ack();
    while (rig.world.phase === "active") await api(commands.rideSkip());
    expect(rig.world.phase).toBe("finished");
    expect(frames.at(-1)?.links.trainer?.phase).toBe("riding");
    const refused = await api(commands.linksIntent(releaseTrainer));
    expect(refused.status === "error" && refused.error.message).toBe(
      "the ride holds the trainer: end the ride first",
    );
    rig.reap();
    expect(frames.at(-1)?.links).toEqual(COLD_LINKS);
    await ack();
    expect(await api(commands.linksIntent(releaseTrainer))).toEqual(NOTHING);
  });

  test("End of a ride that finished on its own waits for its reap: the devices come back cold", async () => {
    const rig = installRide({ phase: "active", keeper: WARM });
    const { frames, ack } = await open();
    await ack();
    while (rig.world.phase === "active") await api(commands.rideSkip());
    expect(await api(commands.rideEnd())).toEqual({
      status: "ok",
      data: { rideId: "ride-1", forced: false },
    });
    expect(frames.at(-1)?.links).toEqual(COLD_LINKS);
    expect(await api(commands.linksIntent(releaseTrainer))).toEqual(NOTHING);
  });

  // The core (keeper/contract_tests.rs `a_page_stalled_across_a_ride_with_an_unlinked_strap_gets_one_frame`).
  test("a page stalled across a ride whose strap missed the load gets the trainer cold, the strap not_found", async () => {
    const keeper: LinkView = {
      trainer: WARM.trainer,
      strap: { phase: "not_found", sample: null },
    };
    const rig = installRide({ phase: "none", keeper, strapHanded: false });
    const { frames, ack } = await open();
    rig.world.phase = "loaded";
    rig.publish();
    rig.world.phase = "active";
    rig.publish();
    await api(commands.rideEnd());
    expect(frames).toHaveLength(1);
    await ack();
    expect(frames).toEqual([
      { seq: 1, links: keeper },
      {
        seq: 3,
        links: {
          trainer: { phase: "cold", sample: null },
          strap: { phase: "not_found", sample: null },
        },
      },
    ]);
  });

  // The ride's end releases its hold whether or not the engine's status frame goes out (core.rs `Core::reap` drops the
  // hold with the session; the status is the engine's).
  test("the hold is released even when no status frame is published", async () => {
    installRide({ phase: "active", keeper: WARM, publishes: false });
    const { frames, ack } = await open();
    await ack();
    await api(commands.rideEnd());
    expect(frames.at(-1)?.links).toEqual(COLD_LINKS);
  });
});

// ADR-0002 §5.1.4 (#597): the link channel is the only source of the keeper's state. An intent's answer carries none;
// its effect is the channel's next frame, sent at once when the last one is acked, otherwise after that ack.
describe("an intent's effect arrives as a frame", () => {
  test("acked: the next frame carries it", async () => {
    installRide({ phase: "none", keeper: WARM });
    const { frames, ack } = await open();
    await ack();
    const answer = await api(
      commands.linksIntent({ kind: "release", reason: "hidden" }),
    );
    expect(answer).toEqual(NOTHING);
    expect(frames).toEqual([
      { seq: 1, links: WARM },
      { seq: 2, links: COLD_LINKS },
    ]);
  });

  test("unacked: it waits for the ack, then arrives as one frame of the latest state", async () => {
    const rig = installRide({ phase: "none", keeper: WARM });
    const { frames, ack } = await open();
    expect(
      await api(commands.linksIntent({ kind: "release", reason: "devices" })),
    ).toEqual(NOTHING);
    rig.setKeeper(SEARCHING);
    expect(frames).toHaveLength(1);
    await ack();
    // the release's `cold` (seq 2) was coalesced into the latest state
    expect(frames).toEqual([
      { seq: 1, links: WARM },
      { seq: 3, links: SEARCHING },
    ]);
  });
});
