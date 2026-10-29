// React bindings of the window's stream client. `StreamProvider` scopes one stream to a loaded ride; the hooks
// only surface what the backend delivered (no derived physiology, #2): `useSubscribe` takes a generated `MetricId`.
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { api, type Result } from "../api";
import type { ApiError, Filter, MetricId, StreamStatus } from "../bindings";
import {
  windowStream,
  type StreamClient,
  type SubscriptionOpts,
  type SubscriptionView,
} from "./client";
import type { Links } from "./state";

const StreamContext = createContext<StreamClient | null>(null);

/** Open the window's stream while mounted. Mount it where a ride is loaded: the stream ends with the ride. */
export function StreamProvider({
  client = windowStream,
  children,
}: {
  client?: StreamClient;
  children: ReactNode;
}) {
  useEffect(() => client.retain(), [client]);
  return <StreamContext value={client}>{children}</StreamContext>;
}

function useClient(): StreamClient {
  const client = useContext(StreamContext);
  if (client === null)
    throw new Error("stream hooks need a <StreamProvider> above them");
  return client;
}

/** What a view asks of a subscription. */
export interface SubscribeRequest {
  /** Delivery rate in session time, 0.1–10 Hz. Required: the core's `rate_hz` is a plain number, there is no default. */
  rateHz: number;
  /** The subscription's own filter chain; none by default. */
  filters?: Filter[];
}

/**
 * The latest sample of `metric`, delivered at `opts.rateHz` through `opts.filters`; unsubscribes on unmount.
 * The hook passes the options on and shows what arrives: the core does the rate limiting and validates the options
 * (ADR-0001 §4.3). `error` is typed: `conflict` beyond 64 subscriptions, `invalid_input` for a rate or filter out of
 * range, `internal` for arguments Tauri refuses.
 */
export function useSubscribe(
  metric: MetricId,
  opts: SubscribeRequest,
): SubscriptionView {
  const client = useClient();
  // Options are plain JSON; a key keeps a re-created literal from resubscribing on every render.
  const key = JSON.stringify({
    rateHz: opts.rateHz,
    filters: opts.filters ?? [],
  });
  const sub = useMemo(
    () => client.subscription(metric, JSON.parse(key) as SubscriptionOpts),
    [client, metric, key],
  );
  useEffect(() => {
    client.add(sub);
    return () => client.remove(sub);
  }, [client, sub]);
  return useSyncExternalStore(
    client.listen,
    () => sub.view,
    () => sub.view,
  );
}

/** The latest ride status; `null` until the first arrives. */
export function useStatus(): StreamStatus | null {
  const client = useClient();
  const get = () => client.snapshot.state.status;
  return useSyncExternalStore(client.listen, get, get);
}

/** The latest state of each device link; a device without an entry has no live link. */
export function useLinks(): Links {
  const client = useClient();
  const get = () => client.snapshot.state.links;
  return useSyncExternalStore(client.listen, get, get);
}

/** The stream's own failure (open, ack, re-query), `null` when healthy. */
export function useStreamError(): ApiError | null {
  const client = useClient();
  const get = () => client.snapshot.error;
  return useSyncExternalStore(client.listen, get, get);
}

/** State of a command made through `useCommand`. */
export interface CommandState<T> {
  /** A call is in flight. */
  pending: boolean;
  /** The latest call's result, kept until the next call finishes. */
  data: T | null;
  /** The latest call's failure, whatever its origin (`toApiError`). */
  error: ApiError | null;
}

/**
 * Run a generated command (`commands.rideStart`, …). `run` resolves with the outcome so a caller can branch on
 * it; only the latest call updates the state.
 */
export function useCommand<A extends unknown[], T>(
  command: (...args: A) => Promise<Result<T>>,
): CommandState<T> & {
  run: (
    ...args: A
  ) => Promise<
    { status: "ok"; data: T } | { status: "error"; error: ApiError }
  >;
} {
  const [state, setState] = useState<CommandState<T>>({
    pending: false,
    data: null,
    error: null,
  });
  const latest = useRef(0);
  const run = useCallback(
    async (...args: A) => {
      const call = ++latest.current;
      setState((s) => ({ ...s, pending: true }));
      const outcome = await api(command(...args));
      if (call === latest.current)
        setState(
          outcome.status === "ok"
            ? { pending: false, data: outcome.data, error: null }
            : { pending: false, data: null, error: outcome.error },
        );
      return outcome;
    },
    [command],
  );
  return { ...state, run };
}
