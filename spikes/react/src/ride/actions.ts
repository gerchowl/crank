import { backend } from "../backend";

/** Y / N: answer the most recent pending nudge. */
export function respondLatest(accept: boolean): boolean {
  const pending = backend.coach.decisions().filter((n) => n.status === "pending");
  const n = pending[pending.length - 1];
  if (!n) return false;
  backend.coach.respond(n.callId, accept);
  return true;
}

