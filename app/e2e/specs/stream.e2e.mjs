// Stream e2e (#89, issue criterion 6): on the real desktop app with the simulated trainer, subscribe to power,
// cadence and hr, freeze the webview for 1 s, resume, and check that no status is lost. Gated like every spec here:
// it runs only under `just e2e` (GUI session, HIL, #35/#261), never in CI or `pnpm test`.
//
// This drives the stream protocol through raw IPC, because no production view mounts the hooks yet (the ride
// screen, #96). It proves what the client depends on: after a stall the core's durable state (`ride_status`,
// `stream_snapshot`) agrees with the last status a frame carried, and frames resume with a `resync` or a gap
// the client turns into a re-query. The same freeze through `StreamProvider` follows with #96. The coach-decisions
// half of the criterion is split out to #309 (gated on the nudges lane, #243).
import { browser, expect } from "@wdio/globals";

describe("stream", () => {
  it("loses no status across a 1 s webview freeze", async () => {
    const out = await browser.executeAsync((done) => {
      const internals = window.__TAURI_INTERNALS__;
      const invoke = internals.invoke;
      const frames = [];
      const id = internals.transformCallback((raw) => {
        if ("message" in raw) frames.push(raw.message);
      });
      const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
      (async () => {
        await invoke("ride_load", { workoutId: null });
        await invoke("ride_start");
        const stream = await invoke("open_stream", {
          channel: `__CHANNEL__:${id}`,
        });
        for (const metric of ["power", "cadence", "hr"])
          await invoke("stream_subscribe", {
            stream,
            metric,
            opts: { rateHz: 4, filters: [] },
          });
        // The frames' own ack: a window of three would otherwise hold the core back for the whole test.
        let acked = -1;
        const ackAll = () => {
          const last = frames.at(-1);
          if (last && last.seq !== acked) {
            acked = last.seq;
            return invoke("stream_ack", { stream, seq: last.seq });
          }
        };
        const pump = setInterval(ackAll, 50);
        await sleep(1500);
        const before = frames.length;
        // Freeze the page: nothing runs, nothing is acked.
        const until = performance.now() + 1000;
        while (performance.now() < until);
        await sleep(1500);
        clearInterval(pump);
        const view = await invoke("ride_status");
        const snap = await invoke("stream_snapshot", { stream });
        const lastStatus = frames
          .map((f) => f.status)
          .filter(Boolean)
          .at(-1);
        done({
          before,
          after: frames.length,
          state: view.status.state.kind,
          snapState: snap.status?.state.kind,
          frameState: lastStatus?.state.kind,
          subs: snap.samples.length,
        });
      })().catch((e) => done({ error: JSON.stringify(e) }));
    });
    expect(out.error).toBeUndefined();
    expect(out.before).toBeGreaterThan(0);
    // Frames resumed after the freeze, and the three subscriptions are live.
    expect(out.after).toBeGreaterThan(out.before);
    expect(out.subs).toBe(3);
    // No status lost: what the last status frame said is what the durable state says.
    expect(out.snapState).toBe(out.state);
    expect(out.frameState).toBe(out.state);
  });
});
