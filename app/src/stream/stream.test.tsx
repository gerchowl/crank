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
import type { MetricId, SubscribeOpts } from "../bindings";
import { StreamClient } from "./client";
import {
  StreamProvider,
  useCommand,
  useLinks,
  useStatus,
  useStreamError,
  useSubscribe,
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
const useRide = (metric: MetricId = "power", opts?: SubscribeOpts) => ({
  sub: useSubscribe(metric, opts),
  status: useStatus(),
  links: useLinks(),
  error: useStreamError(),
});

/** The stream is open, subscribed and has answered its first re-query. */
const ready = async (backend: ReturnType<typeof installBackend>) => {
  await waitFor(() => expect(backend.liveSubs()).toBeGreaterThan(0));
  await waitFor(() => expect(backend.count("stream_snapshot")).toBe(1));
};

describe("useSubscribe", () => {
  test.each([0.1, 1, 10])(
    "delivers the latest sample at %f Hz and unsubscribes on unmount",
    async (rateHz) => {
      const { backend, wrapper } = setup();
      const opts: SubscribeOpts = { rateHz, filters: [] };
      const { result, unmount } = renderHook(() => useRide("power", opts), {
        wrapper,
      });
      await ready(backend);
      const sub = backend.subOf(0);
      const sent = backend.calls.find((c) => c.cmd === "stream_subscribe");
      expect(sent?.args).toMatchObject({ stream: 7, metric: "power", opts });
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

  test("one stream per window under StrictMode, and an option literal does not resubscribe", async () => {
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

  test("a beyond-64th subscription surfaces a typed conflict and the others stay live", async () => {
    const { backend, wrapper } = setup();
    const Probe = ({ n }: { n: number }) => {
      const { sample, error } = useSubscribe("power");
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
  test("keeps consuming past the ack window: the core holds frames until stream_ack", async () => {
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
    expect(backend.count("stream_ack")).toBeGreaterThan(0);
    // No resync was needed for plain flow control.
    expect(backend.count("stream_snapshot")).toBe(1);
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
    backend.setSample(final);
    const before = backend.count("stream_snapshot");
    frames.forEach((f, i) => {
      if (lose && (i === 2 || i === 3)) backend.drop();
      else if (lose && i === 4)
        act(() => void backend.push({ ...f, samples: [], resync: true }));
      else act(() => void backend.push(f));
    });
    await waitFor(() => expect(result.current.sub.sample).toEqual(final));
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
    await waitFor(() => expect(lossy.extra()).toBe(1));
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
    expect(backend.count("stream_snapshot")).toBe(2);
    expect(backend.count("ride_status")).toBe(2);
    expect(backend.count("device_links")).toBe(2);
  });

  test("seq wraps at u32::MAX without a gap", async () => {
    const { backend, wrapper } = setup();
    renderHook(() => useRide(), { wrapper });
    await ready(backend);
    // The first frame is expected to carry 0 on a fresh stream; start the wrap run by resyncing explicitly.
    act(() => void backend.push({ seq: 0 }));
    const max = 0xffff_ffff;
    act(() => void backend.push({ seq: 1 }));
    expect(backend.count("stream_snapshot")).toBe(1);
    act(() => void backend.push({ seq: max }));
    await waitFor(() => expect(backend.count("stream_snapshot")).toBe(2));
    act(() => void backend.push({ seq: 0 }));
    // 0 follows u32::MAX: no second re-query.
    await Promise.resolve();
    expect(backend.count("stream_snapshot")).toBe(2);
  });

  test("a frame that arrives while the re-query is in flight wins over its answer", async () => {
    const { backend, wrapper } = setup();
    const { result } = renderHook(() => useRide(), { wrapper });
    await ready(backend);
    const sub = backend.subOf(0);
    backend.setSample({ sub, t: 1, v: 1 });
    act(() => void backend.push({ resync: true }));
    act(() => void backend.push({ samples: [{ sub, t: 2, v: 2 }] }));
    await waitFor(() => expect(backend.count("stream_snapshot")).toBe(2));
    await waitFor(() =>
      expect(result.current.sub.sample).toEqual({ sub, t: 2, v: 2 }),
    );
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
