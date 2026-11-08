// The order rule for the link keeper's state (ADR-0002 §5.1.4): a frame of the link channel or a `links_intent` answer
// is applied only if its `seq` is at least the highest applied so far, from either source. An answer can arrive after
// a frame of a newer state, or after the answer of a later intent; equal `seq`s are the same state.
import type { LinkFrame } from "../bindings";

/**
 * A frame of the link channel arrives: the page shows the newer of it and what it applied, and acks the frame whether
 * it applied or dropped it. The ack means "received", not "applied" (#577): a dropped frame that is never acked stalls
 * the channel, whose window is 1. An answer of `links_intent` is not a frame and has no ack.
 */
export async function receiveFrame(
  applied: LinkFrame | null,
  frame: LinkFrame,
  ack: (seq: number) => Promise<unknown>,
): Promise<LinkFrame> {
  const shown = keepNewest(applied, frame);
  await ack(frame.seq);
  return shown;
}

/** What the page shows once `next` arrives, given what it applied last (`null` before anything). */
export const keepNewest = (
  applied: LinkFrame | null,
  next: LinkFrame,
): LinkFrame => (applied !== null && next.seq < applied.seq ? applied : next);
