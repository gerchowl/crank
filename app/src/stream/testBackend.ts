// A stand-in for the core's stream commands behind `mockIPC`, for the stream tests only. It models the rules the
// client depends on (ADR-0001 §4.3): an ack window of 3 frames after which the core holds frames back, a cap of 64
// subscriptions per stream (`conflict`), and durable state (`truth`) that `ride_status`, `device_links` and
// `stream_snapshot` answer from. The mock stays a test double: nothing here is the backend's behaviour to rely on.
import type { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  DeviceLinkInfo,
  LinkChange,
  MetricId,
  StreamFrame,
  StreamSnapshot,
  StreamStatus,
  SubSample,
  SubscribeOpts,
} from "../bindings";

export const ACK_WINDOW = 3;
export const SUB_CAP = 64;

export const status = (over: Partial<StreamStatus> = {}): StreamStatus => ({
  state: { kind: "riding" },
  loaded: true,
  sessionS: 0,
  workoutS: 0,
  blockIndex: 0,
  blockElapsedS: 0,
  lateSamples: 0,
  recordError: null,
  trainerFollowing: "following",
  notFollowingComponent: null,
  ergReleased: false,
  compliance: { countedS: 0, withinS: 0, excludedS: 0 },
  cap: null,
  ...over,
});

export interface Call {
  cmd: string;
  args: Record<string, unknown>;
}

/** Install the mock. `stream` is the id `open_stream` answers with. */
export function installBackend(stream = 7) {
  let channel: Channel<StreamFrame> | null = null;
  let nextSub = 1;
  let nextSeq = 0;
  let unacked: number[] = [];
  const held: StreamFrame[] = [];
  const live = new Map<number, { metric: MetricId; opts: SubscribeOpts }>();
  const calls: Call[] = [];
  const truth = {
    samples: new Map<number, SubSample>(),
    status: status(),
    links: [] as LinkChange[],
  };
  const infos: DeviceLinkInfo[] = [];

  const send = (frame: StreamFrame) => {
    unacked.push(frame.seq);
    channel?.onmessage(frame);
  };
  const flush = () => {
    while (held.length > 0 && unacked.length < ACK_WINDOW) {
      const next = held.shift();
      if (next) send(next);
    }
  };

  const backend = {
    calls,
    truth,
    /** Commands called, by name. */
    count: (cmd: string) => calls.filter((c) => c.cmd === cmd).length,
    /** The sub id of the n-th live subscription. */
    subOf: (n: number) => [...live.keys()][n],
    liveSubs: () => live.size,
    /** Frames held back because the window is full. */
    heldCount: () => held.length,
    /** The frame the core would send now; `seq` is numbered unless given. */
    frame(over: Partial<StreamFrame> = {}): StreamFrame {
      return {
        seq: nextSeq,
        resync: false,
        samples: [],
        status: null,
        links: [],
        ...over,
      };
    },
    /** Send a frame, or hold it while the ack window is full. Returns it. */
    push(over: Partial<StreamFrame> = {}): StreamFrame {
      const frame = backend.frame(over);
      nextSeq = (frame.seq + 1) >>> 0;
      if (unacked.length < ACK_WINDOW) send(frame);
      else held.push(frame);
      return frame;
    },
    /** A frame the page never receives (lost in a stall); the sequence still advances. */
    drop(): void {
      nextSeq = (nextSeq + 1) >>> 0;
    },
    /** Make the core's durable state the given sample for `sub`. */
    setSample(sample: SubSample): void {
      truth.samples.set(sample.sub, sample);
    },
  };

  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    calls.push({ cmd, args });
    switch (cmd) {
      case "open_stream":
        channel = args.channel as Channel<StreamFrame>;
        return stream;
      case "stream_subscribe": {
        if (live.size >= SUB_CAP)
          throw { code: "conflict", message: "too many subscriptions" };
        const id = nextSub++;
        live.set(id, {
          metric: args.metric as MetricId,
          opts: args.opts as SubscribeOpts,
        });
        return id;
      }
      case "stream_unsubscribe":
        live.delete(args.sub as number);
        truth.samples.delete(args.sub as number);
        return null;
      case "stream_ack": {
        const seq = args.seq as number;
        const at = unacked.indexOf(seq);
        // Cumulative; a seq that was never sent is ignored.
        if (at >= 0) unacked = unacked.slice(at + 1);
        flush();
        return null;
      }
      case "stream_snapshot":
        return {
          samples: [...truth.samples.values()],
          status: truth.status,
          links: truth.links,
        } satisfies StreamSnapshot;
      case "ride_status":
        return { loading: false, status: truth.status };
      case "device_links":
        return infos;
      default:
        // Tauri rejects an unknown command with a plain string, not an `ApiError`.
        throw `command ${cmd} not found`;
    }
  });
  return backend;
}
