// A stand-in for the core's stream commands behind `mockIPC`, for the stream tests only. It enforces the core's
// contract, not more permissive: an ack window of 3 frames after which the core holds frames back, a cap of 64
// subscriptions per stream (`conflict`, checked before the rate), `SubscribeOpts.rateHz` a required number in
// 0.1–10 Hz (`null` is refused by serde, `invalid_input` outside the range; pinned by the Rust test
// `subscribe_opts_wire_contract_for_the_ui_mock` in crank-engine), `open_stream` replacing the window's earlier
// streams, and `not_found` for a stream that is not the open one. Durable state (`truth`) is what `ride_status`,
// `device_links` and `stream_snapshot` answer from, read when the call arrives; `hold` delays only the answer.
// With `truth.unresponsive` the engine is wedged: every stream command answers `engine_unresponsive` after its owner
// check, as the core's `bounded` (crank-core core.rs, `REQUEST_TIMEOUT`, #351) does.
import type { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  DeviceLinkInfo,
  Filter,
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
export const MIN_RATE_HZ = 0.1;
export const MAX_RATE_HZ = 10;

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
  userPaused: false,
  trainerLostSinceS: null,
  reconnectAttempt: null,
  softStartTargetW: null,
  lastGapS: null,
  reconnectedAtS: null,
  ...over,
});

export interface Call {
  cmd: string;
  args: Record<string, unknown>;
}

const apiError = (code: string, message: string) => ({ code, message });
/** The core's text for a request its 1 s bound gave up on. */
export const UNRESPONSIVE =
  "the ride engine did not answer in time; End still ends the ride";

/** Install the mock. `firstStream` is the id the first `open_stream` answers with; each later one is the next. */
export function installBackend(firstStream = 7) {
  let channel: Channel<StreamFrame> | null = null;
  let current: number | null = null;
  let nextStream = firstStream;
  let nextSub = 1;
  let nextSeq = 0;
  let unacked: number[] = [];
  const held: StreamFrame[] = [];
  const live = new Map<number, { metric: MetricId; opts: SubscribeOpts }>();
  const calls: Call[] = [];
  const acks: { stream: number; seq: number }[] = [];
  const holds = new Map<string, Promise<void>[]>();
  const truth = {
    samples: new Map<number, SubSample>(),
    status: status(),
    links: [] as LinkChange[],
    /** `device_links`: what the core reports per role. */
    infos: [] as DeviceLinkInfo[],
    /** `stream_snapshot` carries no status and no links (to tell the members of a re-query apart). */
    snapshotBare: false,
    /** The engine is wedged: stream commands answer `engine_unresponsive` (#351). */
    unresponsive: false,
  };

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
  const owned = (stream: unknown) => {
    if (stream !== current) throw apiError("not_found", "unknown stream");
    wedged();
  };
  const wedged = () => {
    if (truth.unresponsive) throw apiError("engine_unresponsive", UNRESPONSIVE);
  };

  const backend = {
    calls,
    truth,
    /** Acks the core accepted, in order. */
    acks,
    /** Commands called, by name. */
    count: (cmd: string) => calls.filter((c) => c.cmd === cmd).length,
    /** The sub id of the n-th live subscription. */
    subOf: (n: number) => [...live.keys()][n],
    liveSubs: () => live.size,
    /** Frames held back because the window is full. */
    heldCount: () => held.length,
    /** The id of the open stream. */
    stream: () => current,
    /**
     * Delay the answer of the next call of `cmd` until the returned function is called. Its effect on the core
     * (and for queries the state it reads) is at the call, as for a slow transport.
     */
    hold(cmd: string): () => void {
      let release = () => {};
      const gate = new Promise<void>((resolve) => (release = resolve));
      holds.set(cmd, [...(holds.get(cmd) ?? []), gate]);
      return release;
    },
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

  const handle = (cmd: string, args: Record<string, unknown>): unknown => {
    switch (cmd) {
      case "open_stream": {
        wedged();
        // The window's earlier streams, and their subscriptions, are dropped.
        live.clear();
        truth.samples.clear();
        held.length = 0;
        unacked = [];
        nextSeq = 0;
        channel = args.channel as Channel<StreamFrame>;
        current = nextStream++;
        return current;
      }
      case "stream_subscribe": {
        owned(args.stream);
        const opts = args.opts as { rateHz?: unknown; filters?: unknown };
        // serde: `rate_hz: f64` accepts no `null`; Tauri rejects the call with a plain string.
        if (typeof opts?.rateHz !== "number" || !Array.isArray(opts.filters))
          throw "invalid args `opts` for command `stream_subscribe`: invalid type: null, expected f64";
        if (live.size >= SUB_CAP)
          throw apiError("conflict", "too many subscriptions");
        if (!(opts.rateHz >= MIN_RATE_HZ && opts.rateHz <= MAX_RATE_HZ))
          throw apiError("invalid_input", "rate must be within 0.1..=10 Hz");
        const id = nextSub++;
        live.set(id, {
          metric: args.metric as MetricId,
          opts: { rateHz: opts.rateHz, filters: opts.filters as Filter[] },
        });
        return id;
      }
      case "stream_unsubscribe": {
        owned(args.stream);
        if (!live.delete(args.sub as number))
          throw apiError("not_found", "unknown subscription");
        truth.samples.delete(args.sub as number);
        return null;
      }
      case "stream_ack": {
        owned(args.stream);
        const seq = args.seq as number;
        acks.push({ stream: args.stream as number, seq });
        const at = unacked.indexOf(seq);
        // Cumulative; a seq that was never sent is ignored.
        if (at >= 0) unacked = unacked.slice(at + 1);
        flush();
        return null;
      }
      case "stream_snapshot":
        owned(args.stream);
        return {
          samples: [...truth.samples.values()],
          status: truth.snapshotBare ? null : truth.status,
          links: truth.snapshotBare ? [] : truth.links,
        } satisfies StreamSnapshot;
      case "ride_status":
        return { loading: false, status: truth.status };
      case "device_links":
        return truth.infos;
      default:
        // Tauri rejects an unknown command with a plain string, not an `ApiError`.
        throw `command ${cmd} not found`;
    }
  };

  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    calls.push({ cmd, args });
    let result: unknown;
    let failure: { error: unknown } | null = null;
    try {
      result = handle(cmd, args);
    } catch (error) {
      failure = { error };
    }
    const gate = holds.get(cmd)?.shift();
    const settle = () => {
      if (failure) throw failure.error;
      return result;
    };
    return gate ? gate.then(settle) : settle();
  });
  return backend;
}
