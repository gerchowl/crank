// One window's stream client (ADR-0001 §4.3): a single `Channel<StreamFrame>`, the subscriptions registered on it,
// the ack flow and the re-query on `resync` or a `seq` gap. Framework-free; `hooks.tsx` binds it to React.
//
// Lifecycle: `retain()` opens the stream (`open_stream` needs a loaded ride and the stream ends with that ride, so
// mount one `StreamProvider` per loaded ride). The last `release()` forgets the stream; there is no close command,
// the core drops it when the window opens its next stream, reloads or is destroyed.
//
// To add a re-query member (e.g. `coach_decisions`, #309): add its call to `Requery.run` below and its pure
// `apply…` function to `state.ts`. To add a frame lane, see the header of `state.ts`.
import { Channel } from "@tauri-apps/api/core";
import { api } from "../api";
import {
  commands,
  type ApiError,
  type MetricId,
  type StreamFrame,
  type StreamId,
  type SubId,
  type SubSample,
  type SubscribeOpts,
} from "../bindings";
import {
  EMPTY_STATE,
  applyDeviceLinks,
  applyFrame,
  applyRideStatus,
  applySnapshot,
  newTouched,
  type StreamState,
  type Touched,
} from "./state";

/** What a subscription shows. Replaced on every change. */
export interface SubscriptionView {
  /** Latest sample of the subscription; `null` before the first. `v` is `null` for a gap. */
  readonly sample: SubSample | null;
  /** Why the subscription is not live: `conflict` beyond 64 subscriptions, an invalid rate or filter, no ride. */
  readonly error: ApiError | null;
}

/** What the client shows besides subscriptions. Replaced on every change. */
export interface ClientSnapshot {
  readonly state: StreamState;
  /** The last failure of the stream itself (open, ack, re-query); cleared by the next successful re-query. */
  readonly error: ApiError | null;
}

const IDLE_VIEW: SubscriptionView = { sample: null, error: null };

/** One subscription of the window. Made by `StreamClient.subscription`, started by `add` and ended by `remove`. */
export class Subscription {
  /** The current view; a new object whenever it changes. */
  view: SubscriptionView = IDLE_VIEW;
  /** The core's id while live. */
  id: SubId | null = null;
  /** Bumped by `remove`, so an answer to an earlier `add` is recognised as stale. */
  attempt = 0;
  constructor(
    readonly metric: MetricId,
    readonly opts: SubscribeOpts,
  ) {}
}

/** The re-query after a `resync` or a `seq` gap (ADR-0001 §4.3): the durable state, read once per batch. */
class Requery {
  private running = false;
  private again = false;
  constructor(private readonly client: StreamClient) {}

  /** Start a batch; one already in flight is followed by exactly one more (its answer may predate the trigger). */
  request(): void {
    if (this.running) {
      this.again = true;
      return;
    }
    this.running = true;
    void this.loop();
  }

  private async loop(): Promise<void> {
    try {
      do {
        this.again = false;
        await this.run();
      } while (this.again);
    } finally {
      this.running = false;
    }
  }

  private async run(): Promise<void> {
    const gen = this.client.generation;
    const stream = this.client.streamId;
    if (stream === null) return;
    const touched = this.client.beginTouched();
    // The one place that lists what is re-read.
    const [status, links, snapshot] = await Promise.all([
      api(commands.rideStatus()),
      api(commands.deviceLinks()),
      api(commands.streamSnapshot(stream)),
    ]);
    if (gen !== this.client.generation) return;
    // A failed member does not discard the others; the first failure is the stream's error.
    const failed = [status, links, snapshot].find((r) => r.status === "error");
    this.client.update(
      (s) => {
        let next = s;
        if (status.status === "ok")
          next = applyRideStatus(next, status.data, touched);
        if (links.status === "ok")
          next = applyDeviceLinks(next, links.data, touched);
        if (snapshot.status === "ok")
          next = applySnapshot(next, snapshot.data, touched);
        return next;
      },
      failed?.status === "error" ? failed.error : null,
    );
  }
}

/** The stream of one window. */
export class StreamClient {
  private snap: ClientSnapshot = { state: EMPTY_STATE, error: null };
  private readonly listeners = new Set<() => void>();
  private readonly subs = new Set<Subscription>();
  private readonly requery = new Requery(this);
  private touched: Touched = newTouched();
  private retained = 0;
  private gen = 0;
  private stream: StreamId | null = null;
  private expected = 0;
  private ackWanted: number | null = null;
  private ackSent: number | null = null;
  private acking = false;

  /** Bumped by every open and close; an answer from an earlier one is dropped. */
  get generation(): number {
    return this.gen;
  }
  /** The open stream's id, `null` while opening or closed. */
  get streamId(): StreamId | null {
    return this.stream;
  }
  /** The current snapshot; a new object whenever it changes. */
  get snapshot(): ClientSnapshot {
    return this.snap;
  }

  /** `useSyncExternalStore`'s subscribe. */
  readonly listen = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => void this.listeners.delete(listener);
  };

  private emit(): void {
    for (const l of this.listeners) l();
  }

  /** Mark a consumer; the first one opens the stream. Call the returned function to release. */
  retain(): () => void {
    this.retained += 1;
    if (this.retained === 1) void this.open();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.retained -= 1;
      // A remount in the same task (StrictMode, a re-keyed provider) keeps the stream.
      if (this.retained === 0)
        queueMicrotask(() => this.retained === 0 && this.close());
    };
  }

  private async open(): Promise<void> {
    const gen = ++this.gen;
    this.expected = 0;
    this.ackWanted = this.ackSent = null;
    this.acking = false;
    const channel = new Channel<StreamFrame>();
    channel.onmessage = (frame) => this.onFrame(gen, frame);
    const opened = await api(commands.openStream(channel));
    if (gen !== this.gen) return;
    if (opened.status === "error") {
      this.fail(opened.error);
      return;
    }
    this.stream = opened.data;
    this.pumpAck();
    for (const sub of this.subs) void this.attach(sub);
    this.requery.request();
  }

  private close(): void {
    this.gen += 1;
    this.stream = null;
    this.ackWanted = this.ackSent = null;
    this.acking = false;
    this.snap = { state: EMPTY_STATE, error: null };
    for (const sub of this.subs) {
      sub.id = null;
      sub.view = IDLE_VIEW;
    }
    this.emit();
  }

  private fail(error: ApiError): void {
    this.snap = { ...this.snap, error };
    for (const sub of this.subs)
      if (sub.id === null) sub.view = { sample: null, error };
    this.emit();
  }

  /** Build a subscription without side effects; `add` starts it. */
  subscription(metric: MetricId, opts: SubscribeOpts): Subscription {
    return new Subscription(metric, opts);
  }

  /** Register `sub` with the core as soon as the stream is open. */
  add(sub: Subscription): void {
    this.subs.add(sub);
    if (this.stream !== null) void this.attach(sub);
    else if (this.snap.error !== null) this.fail(this.snap.error);
  }

  /** End `sub`; its sample is dropped. */
  remove(sub: Subscription): void {
    this.subs.delete(sub);
    sub.attempt += 1;
    const { id } = sub;
    sub.id = null;
    sub.view = IDLE_VIEW;
    if (id === null) return;
    this.update((s) => {
      if (!s.samples.has(id)) return s;
      const samples = new Map(s.samples);
      samples.delete(id);
      return { ...s, samples };
    }, null);
    const stream = this.stream;
    if (stream !== null)
      void api(commands.streamUnsubscribe(stream, id)).then((r) => {
        // `not_found` is the stream having ended with its ride: the subscription is gone with it.
        if (r.status === "error" && r.error.code !== "not_found")
          this.fail(r.error);
      });
  }

  private async attach(sub: Subscription): Promise<void> {
    const stream = this.stream;
    if (stream === null) return;
    const attempt = sub.attempt;
    const r = await api(commands.streamSubscribe(stream, sub.metric, sub.opts));
    if (
      attempt !== sub.attempt ||
      !this.subs.has(sub) ||
      stream !== this.stream
    ) {
      // Removed (or the stream closed) while the answer was on its way: end it on the stream it was made on, which
      // the core keeps until the window opens its next one. A `not_found` there means it is gone already.
      if (r.status === "ok")
        void api(commands.streamUnsubscribe(stream, r.data));
      return;
    }
    if (r.status === "error") {
      sub.view = { sample: null, error: r.error };
    } else {
      sub.id = r.data;
      sub.view = {
        sample: this.snap.state.samples.get(r.data) ?? null,
        error: null,
      };
    }
    this.emit();
  }

  /** Begin a re-query: what frames touch from now on wins over its answer. */
  beginTouched(): Touched {
    this.touched = newTouched();
    return this.touched;
  }

  /** Replace the state through `fn`, refresh the views of subscriptions whose sample changed and notify once. */
  update(fn: (s: StreamState) => StreamState, error: ApiError | null): void {
    const before = this.snap.state;
    const state = fn(before);
    this.snap = { state, error };
    if (state.samples !== before.samples)
      for (const sub of this.subs) {
        if (sub.id === null) continue;
        const sample = state.samples.get(sub.id) ?? null;
        if (sample !== sub.view.sample) sub.view = { sample, error: null };
      }
    this.emit();
  }

  private onFrame(gen: number, frame: StreamFrame): void {
    if (gen !== this.gen) return;
    // `seq` wraps, so only equality with the next expected value is meaningful.
    const gap = frame.seq !== this.expected;
    this.expected = (frame.seq + 1) >>> 0;
    this.update((s) => applyFrame(s, frame, this.touched), this.snap.error);
    this.ackWanted = frame.seq;
    this.pumpAck();
    if (frame.resync || gap) this.requery.request();
  }

  /** Acks are cumulative: keep one in flight and send the latest `seq` wanted, never a queue of them. */
  private pumpAck(): void {
    const stream = this.stream;
    if (stream === null || this.acking || this.ackWanted === null) return;
    if (this.ackWanted === this.ackSent) return;
    const gen = this.gen;
    const seq = this.ackWanted;
    this.acking = true;
    void api(commands.streamAck(stream, seq)).then((r) => {
      // An answer from an earlier stream must not touch this one's ack state (`open` and `close` reset it).
      if (gen !== this.gen) return;
      this.acking = false;
      this.ackSent = seq;
      if (r.status === "error") this.fail(r.error);
      this.pumpAck();
    });
  }
}

/** The window's client. Nothing touches the core until a `StreamProvider` retains it. */
export const windowStream = new StreamClient();
