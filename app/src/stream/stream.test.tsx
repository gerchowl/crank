// @vitest-environment jsdom
import {
  act,
  cleanup,
  render,
  renderHook,
  screen,
  waitFor,
} from "@testing-library/react";
import { clearMocks } from "@tauri-apps/api/mocks";
import { StrictMode, type ReactNode } from "react";
import { afterEach, describe, expect, test } from "vitest";
import { commands } from "../bindings";
import type { LinkChange, MetricId } from "../bindings";
import { StreamClient } from "./client";
import {
  StreamProvider,
  useCommand,
  useLinks,
  useStatus,
  useStreamError,
  useSubscribe,
  type SubscribeRequest,
} from "./hooks";
import { ACK_WINDOW, SUB_CAP, installBackend, status } from "./testBackend";

afterEach(() => {
  cleanup();
  clearMocks();
});

const setup = (strict = false) => {
  const backend = installBackend();
  const client = new StreamClient();
  const wrapper = ({ children }: { children: ReactNode }) => {
    const tree = <StreamProvider client={client}>{children}</StreamProvider>;
    return strict ? <StrictMode>{tree}</StrictMode> : tree;
  };
  return { backend, client, wrapper };
};

/** The three hooks a ride view reads, as one value. */
const useRide = (
  metric: MetricId = "power",
  opts: SubscribeRequest = { rateHz: 4 },
) => ({
  sub: useSubscribe(metric, opts),
  status: useStatus(),
  links: useLinks(),
  error: useStreamError(),
});

/** Let anything still in flight land: for asserting that something does *not* happen (a late second batch). */
const settle = () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 40));
  });

/** The stream is open, subscribed and has answered its first re-query. */
const ready = async (backend: ReturnType<typeof installBackend>) => {
  await waitFor(() => expect(backend.liveSubs()).toBeGreaterThan(0));
  await waitFor(() => expect(backend.count("stream_snapshot")).toBe(1));
};

describe("useSubscribe", () => {
  test.each([0.1, 1, 10])(
    "passes a %f Hz request on to the core, shows the latest sample and unsubscribes on unmount",
    async (rateHz) => {
      // The core does the rate limiting (hub.rs `period_s`); the hook forwards the request and shows what arrives.
      const { backend, wrapper } = setup();
      const { result, unmount } = renderHook(
        () => useRide("power", { rateHz }),
        { wrapper },
      );
      await ready(backend);
      const sub = backend.subOf(0);
      const sent = backend.calls.find((c) => c.cmd === "stream_subscribe");
      expect(sent?.args).toMatchObject({
        stream: 7,
        metric: "power",
        opts: { rateHz, filters: [] },
      });
      expect(result.current.sub.sample).toBeNull();

      act(() => {
        backend.push({ samples: [{ sub, t: 1, v: 100 }] });
        backend.push({ samples: [{ sub, t: 2, v: 110 }] });
      });
      expect(result.current.sub.sample).toEqual({ sub, t: 2, v: 110 });

      unmount();
      await waitFor(() => expect(backend.liveSubs()).toBe(0));
      expect(backend.count("stream_unsubscribe")).toBe(1);
    },
  );

  test("a rate the core refuses surfaces its typed invalid_input", async () => {
    const { backend, wrapper } = setup();
    const { result } = renderHook(() => useRide("power", { rateHz: 11 }), {
      wrapper,
    });
    await waitFor(() =>
      expect(result.current.sub.error?.code).toBe("invalid_input"),
    );
    expect(backend.liveSubs()).toBe(0);
  });

  test("a wedged engine's open is the stream's typed engine_unresponsive, shown on the subscription too and not retried (#351)", async () => {
    const { backend, wrapper } = setup();
    backend.truth.unresponsive = true;
    const { result } = renderHook(() => useRide(), { wrapper });
    await waitFor(() =>
      expect(result.current.error?.code).toBe("engine_unresponsive"),
    );
    expect(result.current.sub.error?.code).toBe("engine_unresponsive");
    await settle();
    expect(backend.count("open_stream")).toBe(1);
    expect(backend.count("stream_subscribe")).toBe(0);
  });

  test("an engine that wedges after the open: a subscribe's engine_unresponsive becomes the stream's error (#351)", async () => {
    const { backend, wrapper } = setup();
    const first = renderHook(() => useRide(), { wrapper });
    await ready(backend);
    expect(first.result.current.error).toBeNull();
    backend.truth.unresponsive = true;
    const second = renderHook(() => useRide("cadence"), { wrapper });
    await waitFor(() =>
      expect(second.result.current.sub.error?.code).toBe("engine_unresponsive"),
    );
    expect(first.result.current.error?.code).toBe("engine_unresponsive");
    await settle();
    expect(backend.count("stream_subscribe")).toBe(2);
    expect(backend.count("open_stream")).toBe(1);
  });

  test("a gap sample is null, not the previous value", async () => {
    const { backend, wrapper } = setup();
    const { result } = renderHook(() => useRide(), { wrapper });
    await ready(backend);
    const sub = backend.subOf(0);
    act(() => {
      backend.push({ samples: [{ sub, t: 1, v: 100 }] });
      backend.push({ samples: [{ sub, t: 2, v: null }] });
    });
    expect(result.current.sub.sample).toEqual({ sub, t: 2, v: null });
  });

  test("one stream under StrictMode, and an option literal does not resubscribe", async () => {
    const { backend, wrapper } = setup(true);
    const { rerender } = renderHook(
      () => useRide("cadence", { rateHz: 1, filters: [] }),
      { wrapper },
    );
    await ready(backend);
    rerender();
    rerender();
    expect(backend.count("open_stream")).toBe(1);
    expect(backend.liveSubs()).toBe(1);
  });

  test("StrictMode mount, unmount, mount around a slow open attaches once", async () => {
    const { backend, wrapper } = setup(true);
    const open = backend.hold("open_stream");
    renderHook(() => useRide(), { wrapper });
    open();
    await ready(backend);
    await settle();
    expect(backend.count("open_stream")).toBe(1);
    expect(backend.count("stream_subscribe")).toBe(1);
    expect(backend.liveSubs()).toBe(1);
  });

  test("an unmount while the subscribe answer is on its way still ends the subscription", async () => {
    const { backend, wrapper } = setup();
    const answer = backend.hold("stream_subscribe");
    const { unmount } = renderHook(() => useRide(), { wrapper });
    await waitFor(() => expect(backend.count("stream_subscribe")).toBe(1));
    unmount();
    answer();
    await waitFor(() => expect(backend.liveSubs()).toBe(0));
  });

  test("changing the request while a subscribe answer is on its way leaves only the new subscription", async () => {
    const { backend, wrapper } = setup();
    const answer = backend.hold("stream_subscribe");
    const { rerender } = renderHook(
      ({ rateHz }) => useRide("power", { rateHz }),
      { wrapper, initialProps: { rateHz: 4 } },
    );
    await waitFor(() => expect(backend.count("stream_subscribe")).toBe(1));
    rerender({ rateHz: 5 });
    answer();
    await waitFor(() => expect(backend.count("stream_subscribe")).toBe(2));
    await waitFor(() => expect(backend.liveSubs()).toBe(1));
    await settle();
    expect(backend.liveSubs()).toBe(1);
    expect(
      backend.calls.filter((c) => c.cmd === "stream_subscribe").at(-1)?.args,
    ).toMatchObject({
      opts: { rateHz: 5 },
    });
  });

  test("a beyond-64th subscription surfaces a typed conflict and the others stay live", async () => {
    const { backend, wrapper } = setup();
    const Probe = ({ n }: { n: number }) => {
      const { sample, error } = useSubscribe("power", { rateHz: 4 });
      return (
        <i data-testid={`p${n}`}>{error ? error.code : sample ? "s" : "-"}</i>
      );
    };
    render(
      <>
        {Array.from({ length: SUB_CAP + 1 }, (_, n) => (
          <Probe key={n} n={n} />
        ))}
      </>,
      { wrapper },
    );
    await waitFor(() =>
      expect(screen.getAllByText("conflict")).toHaveLength(1),
    );
    expect(backend.liveSubs()).toBe(SUB_CAP);
    expect(screen.getAllByText("-")).toHaveLength(SUB_CAP);
  });
});

describe("ack", () => {
  test("keeps consuming past the ack window, acking exactly the last seq applied", async () => {
    const { backend, wrapper } = setup();
    const { result } = renderHook(() => useRide(), { wrapper });
    await ready(backend);
    const sub = backend.subOf(0);
    const N = ACK_WINDOW * 4;
    for (let i = 1; i <= N; i++)
      act(() => {
        backend.push({ samples: [{ sub, t: i, v: i }] });
      });
    // Every frame got through although the window holds only 3: the client acked as it applied.
    await waitFor(() => expect(result.current.sub.sample?.v).toBe(N));
    expect(backend.heldCount()).toBe(0);
    // Frames are seq 0..N-1; the ack names the last one applied, not the one before.
    await waitFor(() => expect(backend.acks.at(-1)?.seq).toBe(N - 1));
    // No resync was needed for plain flow control.
    expect(backend.count("stream_snapshot")).toBe(1);
  });

  test("an ack still in flight across close and reopen does not stall the new stream", async () => {
    const { backend, client, wrapper } = setup();
    const first = renderHook(() => useRide(), { wrapper });
    await ready(backend);
    const oldAck = backend.hold("stream_ack");
    act(() => void backend.push({}));
    await waitFor(() => expect(backend.count("stream_ack")).toBe(1));
    first.unmount();
    await waitFor(() => expect(client.streamId).toBeNull());

    renderHook(() => useRide(), { wrapper });
    await waitFor(() => expect(client.streamId).toBe(8));
    act(() => {
      for (let i = 0; i < ACK_WINDOW; i++) backend.push({});
    });
    oldAck();
    // The window of the new stream is full: only an ack for it lets the core send more.
    await waitFor(() =>
      expect(backend.acks).toContainEqual({ stream: 8, seq: ACK_WINDOW - 1 }),
    );
    act(() => void backend.push({}));
    expect(backend.heldCount()).toBe(0);
  });
});

describe("resync", () => {
  /** Frames 0..4 for one sub, status change in frame 1; `lose` frames 2-3 and flag frame 4 `resync`. */
  async function run(lose: boolean) {
    const { backend, wrapper } = setup();
    const { result } = renderHook(() => useRide(), { wrapper });
    await ready(backend);
    const sub = backend.subOf(0);
    const paused = status({ state: { kind: "paused", reason: "user" } });
    const final = { sub, t: 4, v: 140 };
    const frames = [
      { samples: [{ sub, t: 0, v: 100 }] },
      { samples: [{ sub, t: 1, v: 110 }], status: paused },
      {
        samples: [{ sub, t: 2, v: 120 }],
        links: [{ device: "hr" as const, state: "lost" as const }],
      },
      { samples: [{ sub, t: 3, v: 130 }] },
      { samples: [final] },
    ];
    // The core's durable state is what the whole run produced.
    backend.truth.status = paused;
    backend.truth.links = [{ device: "hr", state: "lost" }];
    backend.truth.infos = [{ role: "heart_rate", device: null, link: "lost" }];
    backend.setSample(final);
    const before = backend.count("stream_snapshot");
    frames.forEach((f, i) => {
      if (lose && (i === 2 || i === 3)) backend.drop();
      else if (lose && i === 4)
        act(() => void backend.push({ ...f, samples: [], resync: true }));
      else act(() => void backend.push(f));
    });
    await waitFor(() => expect(result.current.sub.sample).toEqual(final));
    await settle();
    return {
      backend,
      result,
      extra: () => backend.count("stream_snapshot") - before,
    };
  }

  test("a resync frame triggers exactly one re-query batch and ends in the no-gap state", async () => {
    const clean = await run(false);
    const expected = clean.result.current;
    expect(clean.extra()).toBe(0);
    cleanup();
    clearMocks();

    const lossy = await run(true);
    // `run` settled: a late second batch would show here.
    expect(lossy.extra()).toBe(1);
    expect(lossy.result.current.status).toEqual(expected.status);
    expect(lossy.result.current.links).toEqual(expected.links);
    // One batch: each member once (plus the one at open).
    expect(lossy.backend.count("ride_status")).toBe(2);
    expect(lossy.backend.count("device_links")).toBe(2);
    expect(lossy.backend.count("stream_snapshot")).toBe(2);
  });

  test("a seq gap without a resync flag takes the same path", async () => {
    const { backend, wrapper } = setup();
    const { result } = renderHook(() => useRide(), { wrapper });
    await ready(backend);
    const sub = backend.subOf(0);
    act(() => void backend.push({ samples: [{ sub, t: 0, v: 1 }] }));
    backend.drop();
    backend.setSample({ sub, t: 5, v: 9 });
    act(() => void backend.push({ samples: [] }));
    await waitFor(() => expect(result.current.sub.sample?.v).toBe(9));
    await settle();
    expect(backend.count("stream_snapshot")).toBe(2);
    expect(backend.count("ride_status")).toBe(2);
    expect(backend.count("device_links")).toBe(2);
  });

  test("seq wraps at u32::MAX without a gap", async () => {
    const { backend, wrapper } = setup();
    renderHook(() => useRide(), { wrapper });
    await ready(backend);
    act(() => void backend.push({ seq: 0 }));
    act(() => void backend.push({ seq: 1 }));
    await settle();
    expect(backend.count("stream_snapshot")).toBe(1);
    // A jump to u32::MAX is a gap; 0 after it is not.
    act(() => void backend.push({ seq: 0xffff_ffff }));
    await waitFor(() => expect(backend.count("stream_snapshot")).toBe(2));
    act(() => void backend.push({ seq: 0 }));
    await settle();
    expect(backend.count("stream_snapshot")).toBe(2);
  });

  test("a resync that arrives during a batch causes exactly one follow-up batch", async () => {
    const { backend, wrapper } = setup();
    renderHook(() => useRide(), { wrapper });
    await ready(backend);
    const answer = backend.hold("stream_snapshot");
    act(() => void backend.push({ resync: true }));
    await waitFor(() => expect(backend.count("stream_snapshot")).toBe(2));
    act(() => void backend.push({ resync: true }));
    act(() => void backend.push({ resync: true }));
    await settle();
    // Not started while the first is in flight ...
    expect(backend.count("stream_snapshot")).toBe(2);
    answer();
    // ... and then one, not one per trigger.
    await waitFor(() => expect(backend.count("stream_snapshot")).toBe(3));
    await settle();
    expect(backend.count("stream_snapshot")).toBe(3);
    expect(backend.count("ride_status")).toBe(3);
    expect(backend.count("device_links")).toBe(3);
  });

  test("a frame that arrives during a batch wins over the batch's older answer", async () => {
    const { backend, wrapper } = setup();
    const { result } = renderHook(() => useRide(), { wrapper });
    await ready(backend);
    const sub = backend.subOf(0);
    // The answers are read now and delivered late.
    backend.truth.status = status({
      state: { kind: "paused", reason: "user" },
    });
    backend.truth.links = [{ device: "hr", state: "lost" }];
    backend.truth.infos = [{ role: "heart_rate", device: null, link: "lost" }];
    backend.setSample({ sub, t: 1, v: 1 });
    const answers = ["ride_status", "device_links", "stream_snapshot"].map(
      (c) => backend.hold(c),
    );
    act(() => void backend.push({ resync: true }));
    await waitFor(() => expect(backend.count("stream_snapshot")).toBe(2));
    const riding = status();
    act(
      () =>
        void backend.push({
          samples: [{ sub, t: 2, v: 2 }],
          status: riding,
          links: [{ device: "hr", state: "ready" }],
        }),
    );
    answers.forEach((release) => release());
    await settle();
    expect(result.current.sub.sample).toEqual({ sub, t: 2, v: 2 });
    expect(result.current.status).toEqual(riding);
    expect(result.current.links).toEqual({ hr: "ready" });
  });

  test("ride_status and device_links answers are applied, with roles mapped to devices", async () => {
    const { backend, wrapper } = setup();
    const { result } = renderHook(() => useRide(), { wrapper });
    await ready(backend);
    const links: LinkChange[] = [{ device: "trainer", state: "ready" }];
    act(() => void backend.push({ links }));
    expect(result.current.links).toEqual({ trainer: "ready" });

    // The snapshot says nothing about status and links, so only the other two members can supply them.
    backend.truth.snapshotBare = true;
    backend.truth.status = status({ state: { kind: "finished" } });
    backend.truth.infos = [
      { role: "trainer", device: null, link: null },
      { role: "heart_rate", device: null, link: "connecting" },
    ];
    act(() => void backend.push({ resync: true }));
    await waitFor(() =>
      expect(result.current.status?.state).toEqual({ kind: "finished" }),
    );
    // A role without a live link has no entry; `heart_rate` is the stream's `hr`.
    expect(result.current.links).toEqual({ hr: "connecting" });
  });

  test("a re-query answer that arrives after the stream closed is dropped", async () => {
    const { backend, client, wrapper } = setup();
    const { unmount } = renderHook(() => useRide(), { wrapper });
    await ready(backend);
    backend.truth.status = status({
      state: { kind: "paused", reason: "user" },
    });
    const answer = backend.hold("stream_snapshot");
    act(() => void backend.push({ resync: true }));
    await waitFor(() => expect(backend.count("stream_snapshot")).toBe(2));
    unmount();
    await waitFor(() => expect(client.streamId).toBeNull());
    answer();
    await settle();
    expect(client.snapshot.state.status).toBeNull();
  });
});

describe("useCommand", () => {
  test("reports pending, data and a typed error", async () => {
    installBackend();
    const { result } = renderHook(() => useCommand(commands.rideStatus));
    expect(result.current.pending).toBe(false);
    let outcome: Awaited<ReturnType<typeof result.current.run>> | undefined;
    await act(async () => {
      outcome = await result.current.run();
    });
    expect(outcome?.status).toBe("ok");
    expect(result.current.data?.loading).toBe(false);

    // Tauri rejects an unknown command with a plain string; the hook still gives an ApiError.
    const bad = renderHook(() => useCommand(commands.rideStart));
    await act(async () => {
      await bad.result.current.run();
    });
    expect(bad.result.current.error?.code).toBe("internal");
  });
});
