// Cold start (docs/design/BRAND.md §Start screen): the app is "booting" from process launch until the core
// reports ready. The prototype's mock core is synchronous, so a boot is ready at once and there is no start
// screen. The harness simulates a slow or failing core with a query parameter, driven by timers (so a
// fake clock controls it):
//   ?boot=<ms>        the core becomes ready after <ms>
//   ?boot=fail[:<ms>] the core fails after <ms> (default 0), on every attempt
import { createStore } from "./store";

export type Boot = { phase: "booting"; attempt: number } | { phase: "ready" } | { phase: "failed"; detail: string };

/** What the simulated core does on one attempt; null = the instant, always-ready mock. */
type Plan = { ms: number; fail: boolean } | null;

function parsePlan(raw: string | null): Plan {
  if (raw === null) return null;
  const m = /^(fail)?:?(\d*)$/.exec(raw);
  if (!m || (!m[1] && m[2] === "")) throw new Error(`bad ?boot=${raw} (want <ms>, fail or fail:<ms>)`);
  return { ms: Number(m[2] || 0), fail: m[1] === "fail" };
}

const plan = parsePlan(new URLSearchParams(location.search).get("boot"));

/** Why the simulated core did not come up (stands in for the real handshake / persisted-state error). */
export const SIMULATED_FAILURE = "its saved state could not be opened";

export const boot = createStore<Boot>(plan ? { phase: "booting", attempt: 1 } : { phase: "ready" });

let attempts = 0;
function attempt() {
  if (!plan) return;
  const { ms, fail } = plan;
  boot.set({ phase: "booting", attempt: ++attempts });
  setTimeout(() => boot.set(fail ? { phase: "failed", detail: SIMULATED_FAILURE } : { phase: "ready" }), ms);
}

/** Begin the first attempt; the cold start is timed from here. */
export const startBoot = attempt;
/** "Try again" on the boot-failure line: a fresh attempt, with its own 1.2 s status clock. */
export const retryBoot = attempt;
