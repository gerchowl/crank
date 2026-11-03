// The window's view of the stream: what frames and re-queries deliver, as plain JSON (no `structuredClone`, no
// proxies, #5). Pure functions, so the ordering rules are testable without a channel.
//
// Adding a lane (ADR-0001 §4.3 nudges, #309) is a change in this file and one line in `client.ts`:
//   1. a field in `StreamState` and `EMPTY_STATE`,
//   2. a branch in `applyFrame` that records what it touched in `Touched`,
//   3. an `apply…` function for its durable re-query, called from `Requery` in `client.ts`.
import type {
  DeviceLinkInfo,
  LinkChange,
  LinkDevice,
  RideStatusView,
  StreamFrame,
  StreamSnapshot,
  StreamStatus,
  SubId,
  SubSample,
} from "../bindings";

/** Latest link state per device; a device without an entry has no live link. */
export type Links = Readonly<Partial<Record<LinkDevice, LinkChange["state"]>>>;

/** Everything the hooks read. Replaced, never mutated, so a reader can hold a reference. */
export interface StreamState {
  /** Latest sample of each live subscription. */
  readonly samples: ReadonlyMap<SubId, SubSample>;
  /** Latest ride status, `null` before the first one arrives. */
  readonly status: StreamStatus | null;
  /** Latest link state per device. */
  readonly links: Links;
}

export const EMPTY_STATE: StreamState = {
  samples: new Map(),
  status: null,
  links: {},
};

/**
 * What frames delivered while a re-query was in flight. A frame is never older than the request, so it wins over
 * the answer for the same item: the answer fills only what no frame touched since the request was sent.
 */
export interface Touched {
  subs: Set<SubId>;
  status: boolean;
  links: Set<LinkDevice>;
}

export const newTouched = (): Touched => ({
  subs: new Set(),
  status: false,
  links: new Set(),
});

/** Apply one frame: samples, then status, then links (everything in a frame precedes the next frame). */
export function applyFrame(
  state: StreamState,
  frame: StreamFrame,
  touched: Touched,
): StreamState {
  let samples = state.samples;
  if (frame.samples.length > 0) {
    const next = new Map(samples);
    for (const s of frame.samples) {
      next.set(s.sub, s);
      touched.subs.add(s.sub);
    }
    samples = next;
  }
  let status = state.status;
  if (frame.status !== null) {
    status = frame.status;
    touched.status = true;
  }
  let links = state.links;
  if (frame.links.length > 0) {
    const next = { ...links };
    for (const l of frame.links) {
      next[l.device] = l.state;
      touched.links.add(l.device);
    }
    links = next;
  }
  return { samples, status, links };
}

/** Apply `stream_snapshot`: latest value of every live subscription, the status and the links. */
export function applySnapshot(
  state: StreamState,
  snap: StreamSnapshot,
  touched: Touched,
): StreamState {
  const live = new Set(snap.samples.map((s) => s.sub));
  const samples = new Map<SubId, SubSample>();
  // The snapshot is authoritative for which subscriptions exist; a frame sample of a subscription the snapshot
  // does not know was delivered after it ended or before it began, and either way is kept only if it was touched.
  for (const [id, s] of state.samples)
    if (live.has(id) || touched.subs.has(id)) samples.set(id, s);
  for (const s of snap.samples)
    if (!touched.subs.has(s.sub)) samples.set(s.sub, s);
  const status =
    touched.status || snap.status === null ? state.status : snap.status;
  const links = { ...state.links };
  for (const l of snap.links)
    if (!touched.links.has(l.device)) links[l.device] = l.state;
  return { samples, status, links };
}

/** Apply `ride_status`: the durable status, which is never `null`. */
export function applyRideStatus(
  state: StreamState,
  view: RideStatusView,
  touched: Touched,
): StreamState {
  return touched.status ? state : { ...state, status: view.status };
}

/** The command roles name the heart-rate strap `heart_rate`, the stream names it `hr`. */
const DEVICE_OF_ROLE = { trainer: "trainer", heart_rate: "hr" } as const;

/** Apply `device_links`: a role without a live link (no ride, or ended) has no entry. */
export function applyDeviceLinks(
  state: StreamState,
  infos: DeviceLinkInfo[],
  touched: Touched,
): StreamState {
  const links: Partial<Record<LinkDevice, LinkChange["state"]>> = {
    ...state.links,
  };
  for (const info of infos) {
    const device = DEVICE_OF_ROLE[info.role];
    if (touched.links.has(device)) continue;
    if (info.link === null) delete links[device];
    else links[device] = info.link;
  }
  return { ...state, links };
}
