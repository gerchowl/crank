// @vitest-environment jsdom
// The link commands of the ride test backend (`rideBackend.ts`) answer and refuse as the core does (ADR-0002 §5.1.3,
// §5.1.4): each refusal below is listed with its core symbol in the backend's header.
import { Channel } from "@tauri-apps/api/core";
import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, test } from "vitest";
import { api } from "../api";
import { commands } from "../bindings";
import type { LinkFrame, LinkView } from "../bindings";
import { keepNewest, receiveFrame } from "./linkOrder";
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
    expect(sleep.status === "ok" && sleep.data.links.trainer?.phase).toBe(
      "riding",
    );
    expect(rig.world.phase).toBe("active");
  });

  test("Release trainer or Sleep unloads an unstarted ride and answers cold", async () => {
    for (const reason of ["rider_release", "sleep"] as const) {
      const rig = installRide({ phase: "loaded", keeper: WARM });
      const done = await api(commands.linksIntent({ kind: "release", reason }));
      expect(done.status === "ok" && done.data.links).toEqual(COLD_LINKS);
      expect(rig.world.phase).toBe("none");
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
      const r = await api(commands.linksIntent(intent));
      expect(r.status === "ok" && r.data.links.strap?.phase).toBe("riding");
    }
    expect(rig.world.phase).toBe("loaded");
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
    expect(discarded.status === "ok" && discarded.data.links).toEqual(
      COLD_LINKS,
    );
    expect(rig.world.phase).toBe("none");
    clearMocks();
    rig = installRide({ phase: "active", keeper: WARM, supersedes: true });
    const kept = await api(
      commands.linksIntent({ kind: "release", reason: "hidden" }),
    );
    expect(kept.status === "ok" && kept.data.links.trainer?.phase).toBe(
      "riding",
    );
    expect(rig.world.phase).toBe("active");
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
    expect(r.status).toBe("ok");
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
  async function open() {
    const frames: LinkFrame[] = [];
    const channel = new Channel<LinkFrame>();
    channel.onmessage = (f) => frames.push(f);
    const opened = await api(commands.linksOpen(channel));
    if (opened.status !== "ok") throw new Error("open failed");
    const ack = () => api(commands.linksAck(opened.data, frames.at(-1)!.seq));
    return { frames, ack, id: opened.data };
  }
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
  // (core.rs `Core::links_intent` (~1248) finds no session: core.rs `Core::end_ride` (~2197) waited for the reap).
  test("Release trainer after End is accepted: the ended ride holds nothing", async () => {
    installRide({ phase: "active", keeper: WARM });
    const { frames, ack } = await open();
    await ack();
    await api(commands.rideEnd());
    const r = await api(commands.linksIntent(releaseTrainer));
    expect(r.status === "ok" && r.data.links).toEqual(COLD_LINKS);
    expect(r.status === "ok" && r.data.seq).toBe(frames.at(-1)?.seq);
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
    const r = await api(commands.linksIntent(releaseTrainer));
    expect(r.status).toBe("ok");
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
    expect((await api(commands.linksIntent(releaseTrainer))).status).toBe("ok");
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

// ADR-0002 §5.1.4: a frame or an answer is applied only if its seq is at least the highest applied so far, from either
// source (keeper/contract_tests.rs `applied_in_arrival_order_the_newest_state_wins_an_answer_after_a_newer_answer_included`).
describe("answers and frames are ordered by seq", () => {
  test("an answer has its state's seq; a later change outranks it", async () => {
    const rig = installRide({ phase: "none", keeper: WARM });
    const frames: LinkFrame[] = [];
    const channel = new Channel<LinkFrame>();
    channel.onmessage = (f) => frames.push(f);
    const opened = await api(commands.linksOpen(channel));
    if (opened.status !== "ok") throw new Error("open failed");
    const answer = await api(commands.linksIntent({ kind: "left" }));
    if (answer.status !== "ok") throw new Error("intent failed");
    expect(answer.data).toEqual(frames.at(-1));
    await api(commands.linksAck(opened.data, answer.data.seq));
    rig.setKeeper(COLD_LINKS);
    const newer = frames.at(-1)!;
    expect(newer.seq).toBeGreaterThan(answer.data.seq);
    expect(keepNewest(newer, answer.data)).toEqual(newer);
    const released = await api(
      commands.linksIntent({ kind: "release", reason: "hidden" }),
    );
    expect(released.status === "ok" && released.data.seq).toBe(newer.seq);
  });

  // #561 round 3: the release's answer waits for its links and arrives after the answer of the `likely` sent after it.
  test("applied in arrival order, an older answer after a newer one never steps back", async () => {
    const rig = installRide({ phase: "none", keeper: WARM });
    const log: { from: string; frame: LinkFrame }[] = [];
    const channel = new Channel<LinkFrame>();
    channel.onmessage = (frame) => log.push({ from: "frame", frame });
    const opened = await api(commands.linksOpen(channel));
    if (opened.status !== "ok") throw new Error("open failed");
    const answered =
      (from: string) => (a: { status: string; data?: LinkFrame }) => {
        if (a.status !== "ok" || !a.data) throw new Error(`${from} failed`);
        log.push({ from, frame: a.data });
      };
    // the first frame is not acked yet: the changes below wait for its ack
    const letGo = rig.hold("links_intent");
    const release = api(
      commands.linksIntent({ kind: "release", reason: "devices" }),
    ).then(answered("release"));
    rig.setKeeper(SEARCHING);
    await api(commands.linksIntent({ kind: "likely", ready: false })).then(
      answered("likely"),
    );
    letGo();
    await release;
    await api(commands.linksAck(opened.data, log[0].frame.seq));
    expect(log.map((a) => [a.from, a.frame.seq])).toEqual([
      ["frame", 1],
      ["likely", 3],
      ["release", 2],
      ["frame", 3],
    ]);
    expect(log[2].frame.links).toEqual(COLD_LINKS);
    const applied: LinkFrame[] = [];
    for (const { frame } of log)
      applied.push(keepNewest(applied.at(-1) ?? null, frame));
    expect(applied.map((f) => f.seq)).toEqual([1, 3, 3, 3]);
    expect(applied.at(-1)?.links).toEqual(SEARCHING);
  });

  // #577: the UI acks every frame it receives, applied or dropped. Answer 12 arrives, then the older frame 11: the page
  // drops 11, still acks it, and the next frame brings it to the keeper's state (keeper/contract_tests.rs
  // `a_dropped_frame_that_is_acked_lets_the_page_converge`).
  test("a frame dropped by the order rule is still acked, and the page converges", async () => {
    const rig = installRide({ phase: "none", keeper: WARM });
    const frames: LinkFrame[] = [];
    const channel = new Channel<LinkFrame>();
    channel.onmessage = (f) => frames.push(f);
    const opened = await api(commands.linksOpen(channel));
    if (opened.status !== "ok") throw new Error("open failed");
    // the channel's first frame is in flight; the answer of a newer state outruns it
    rig.setKeeper(SEARCHING);
    const answer = await api(commands.linksIntent({ kind: "left" }));
    if (answer.status !== "ok") throw new Error("intent failed");
    expect(frames.map((f) => f.seq)).toEqual([1]);
    expect(answer.data.seq).toBeGreaterThan(frames[0].seq);
    const ack = async (seq: number) => {
      await api(commands.linksAck(opened.data, seq));
    };
    let shown = keepNewest(null, answer.data);
    shown = await receiveFrame(shown, frames[0], ack);
    expect(shown).toEqual(answer.data); // the older frame was dropped...
    expect(frames.map((f) => f.seq)).toEqual([1, answer.data.seq]); // ...but acked, so the next frame went out
    rig.setKeeper(WARM);
    shown = await receiveFrame(shown, frames[1], ack);
    expect(frames.map((f) => f.seq)).toEqual([
      1,
      answer.data.seq,
      answer.data.seq + 1,
    ]);
    shown = await receiveFrame(shown, frames[2], ack);
    expect(shown).toEqual(frames.at(-1));
    expect(shown.links).toEqual(WARM);
  });
});
