// @vitest-environment jsdom
// The link commands of the ride test backend (`rideBackend.ts`) answer and refuse as the core does (ADR-0002 §5.1.3,
// §5.1.4): each refusal below is listed with its core symbol in the backend's header.
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
    expect(sleep.status === "ok" && sleep.data.trainer?.phase).toBe("riding");
    expect(rig.world.phase).toBe("active");
  });

  test("Release trainer or Sleep unloads an unstarted ride and answers cold", async () => {
    for (const reason of ["rider_release", "sleep"] as const) {
      const rig = installRide({ phase: "loaded", keeper: WARM });
      const done = await api(commands.linksIntent({ kind: "release", reason }));
      expect(done).toEqual({ status: "ok", data: COLD_LINKS });
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
      expect(r.status === "ok" && r.data.strap?.phase).toBe("riding");
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
    expect(frames).toEqual([
      { seq: 1, links: COLD_LINKS },
      { seq: 2, links: WARM },
    ]);
    const stale = await api(commands.linksAck(opened.data + 1, 2));
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
// then `cold` with no sample, in consecutive seqs.
describe("the ride's hold on the links", () => {
  async function open() {
    const frames: LinkFrame[] = [];
    const channel = new Channel<LinkFrame>();
    channel.onmessage = (f) => frames.push(f);
    const opened = await api(commands.linksOpen(channel));
    if (opened.status !== "ok") throw new Error("open failed");
    const ack = () => api(commands.linksAck(opened.data, frames.at(-1)!.seq));
    return { frames, ack };
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
    expect(frames).toEqual([
      { seq: 1, links: WARM },
      { seq: 2, links: COLD_LINKS },
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
});
