// The MVP walk (#667): the whole #256 critical path driven through the real app's UI in sim mode, asserting what the
// rider sees. The steps follow #667's list; each assertion names its #256 step ("#256: …").
// Only DOM and visible text are asserted (selectors in mvp-selectors.mjs, each with its component), and no test probe
// is read: #656's `window.__crankUi` is not used. The one helper this file puts in the page (`record`) only reads the
// ride screen's slots, so a soft start's ramp and the live tiles can be seen between two WebDriver calls. The one
// `invoke` is step 7's cause: `sim_fault`, the e2e build's test hook (#668, app/src-tauri/src/commands.rs), makes the
// simulated trainer or strap drop out in the real core; what the rider then sees is read from the DOM like the rest.
//
// The ride is the built-in "Sensor check (3 min)" (crates/crank-core/src/library.rs `BUILTIN`): FTP 250 W, 1 min each at
// 50 %, 80 % and 40 %, so its plan targets are 125 W, 200 W and 100 W. The simulator's scan offers "crank sim trainer"
// and "crank sim strap" (crates/crank-core/src/backend.rs `Backend::scan`), and its rider pedals on its own.
//
// Step 9 (the ride is in History after a relaunch) is mvp-relaunch.e2e.mjs. This spec ends by closing the window,
// which quits the app (AGENTS.md "Quitting ends a ride"). The service starts it again on the same data dir for the
// next spec (wdio.conf.mjs runs the specs in order).
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { browser, $, $$, expect } from "@wdio/globals";
import * as sel from "./mvp-selectors.mjs";

const outDir = process.env.CRANK_E2E_OUT ?? "e2e-out";
const WORKOUT = "Sensor check (3 min)";
/** The plan's target per block, W (library.rs `BUILTIN`: 50 %, 80 %, 40 % of 250 W). */
const PLAN_W = [125, 200, 100];
const TRAINER = "crank sim trainer";
const STRAP = "crank sim strap";
/** The waits of this spec. Each one waits for a condition. A slow condition fails with its message; nothing is retried. */
const UI_MS = 10_000;
/** A soft start (5 s, ADR-0002 §7.5) behind the 0 W hold until the first pedalling sample. */
const ENGAGE_MS = 20_000;

/** The text of a node, trimmed. */
const text = async (selector) => (await $(selector).getText()).trim();
/** Waits until the node's text satisfies `ok`, and returns that text. */
async function textUntil(selector, ok, what, timeout = UI_MS) {
  let last = null;
  await browser.waitUntil(
    async () => {
      const el = await $(selector);
      last = (await el.isExisting()) ? (await el.getText()).trim() : null;
      return last !== null && ok(last);
    },
    { timeout, timeoutMsg: `${what} (last text: ${JSON.stringify(last)})` },
  );
  return last;
}
const numeric = (s) => (/^\d+$/.test(s ?? "") ? Number(s) : null);

/** Step 7's losses, ms. The sim reconnects at its first attempt at or after this: 3.5 s (sim/mod.rs `outage`). */
const DOWN_MS = 3_000;
/**
 * The e2e build's test hook (#668): the simulated trainer (`"trainer"`) or strap (`"heart_rate"`) loses its link now for
 * `downMs`, then reconnects (crank-core `Core::sim_fault`). Resolves to `null`, or to the `ApiError` it was refused with.
 */
const simFault = (role, downMs) =>
  browser.executeAsync(
    (role, downMs, done) => {
      window.__TAURI_INTERNALS__
        .invoke("sim_fault", { role, downMs })
        .then(() => done(null), done);
    },
    role,
    downMs,
  );

/**
 * Starts recording the ride screen's live slots into `window.__mvpRecord`. Each change of any of them appends one
 * entry `{ target, plan, power, cadence, hr, block }` (their text). It only reads the DOM, through a MutationObserver
 * on the screen's root.
 */
function record() {
  return browser.execute(() => {
    const SLOTS = ["target", "plan", "power", "cadence", "hr", "block"];
    const root = document.querySelector('[data-testid="ride"]');
    const read = () =>
      Object.fromEntries(
        SLOTS.map((s) => [
          s,
          document.querySelector(`[data-slot="${s}"]`)?.textContent?.trim() ??
            null,
        ]),
      );
    const seen = [read()];
    let last = JSON.stringify(seen[0]);
    new MutationObserver(() => {
      const now = read();
      const key = JSON.stringify(now);
      if (key !== last) {
        last = key;
        seen.push(now);
      }
    }).observe(root, { subtree: true, childList: true, characterData: true });
    window.__mvpRecord = seen;
  });
}
const recorded = () => browser.execute(() => window.__mvpRecord);

/** The distinct consecutive values of one slot in the record. */
function runs(entries, slot) {
  const out = [];
  for (const e of entries) if (out.at(-1) !== e[slot]) out.push(e[slot]);
  return out;
}

describe("MVP walk (#256 critical path, sim)", () => {
  it("1. pairs the trainer and the strap from Settings → Devices", async () => {
    // #256: "connect the trainer and HR strap": Settings → Devices, Scan, then Pair per role, all by the UI's buttons.
    await $(sel.rail("Settings")).click();
    await $(sel.settingsSection("Devices")).click();
    // A scratch data dir: nothing is paired yet.
    for (const role of ["Trainer", "Heart-rate strap"])
      await textUntil(
        sel.pairedRow(role),
        (t) => t.endsWith("Not paired"),
        `${role} was not "Not paired" at the start`,
      );
    await $(sel.scanButton).click();

    await $(sel.pairButton("trainer", TRAINER)).click();
    // #256: the trainer shows as paired: in the Nearby row and in the Paired table.
    await expect($(sel.pairedMark("trainer", TRAINER))).toBeDisplayed({
      wait: UI_MS,
    });
    await expect($(sel.pairedLine("Trainer"))).toBeDisplayed();

    await $(sel.pairButton("strap", STRAP)).click();
    // #256: the strap shows as paired, the trainer still is.
    await expect($(sel.pairedMark("strap", STRAP))).toBeDisplayed({
      wait: UI_MS,
    });
    await expect($(sel.pairedLine("Heart-rate strap"))).toBeDisplayed();
    await expect($(sel.pairedLine("Trainer"))).toBeDisplayed();
  });

  it("2. picks a workout, Ride this, the Ready sheet, Start", async () => {
    // #256: "pick a workout" (Workouts → the row → Ride this) loads the ride and opens the ride screen.
    await $(sel.rail("Workouts")).click();
    await $(sel.workoutOption(WORKOUT)).click();
    await $(sel.rideThis).click();
    // #256: both devices connected for the ride. The Ready sheet's rows say "connected" also when no link has been
    // reported (ReadySheet.tsx `phase` defaults to the load's answer), so the proof is the status bar's pill: its
    // `data-link` is the link the core reported, "none" until it does (Ride.tsx `Pill`).
    await expect($(sel.readySheet("ready"))).toBeDisplayed({ wait: UI_MS });
    await expect($(sel.readyDevice("trainer", "connected"))).toBeDisplayed();
    await expect($(sel.readyDevice("hr", "connected"))).toBeDisplayed();
    for (const name of ["NEO", "HR"])
      await expect($(sel.pill(name))).toHaveAttribute("data-link", "ready", {
        wait: UI_MS,
      });
    // #256: "ride it in ERG": the sheet's plan line says ERG and starts with the warm-up's 1:00.
    await expect($(sel.readyPlan)).toHaveText(/^ERG · starts with 1:00 /);

    // Record from before Start, so the soft start's first values are seen (step 3).
    await record();
    const start = $(sel.key("Start"));
    await expect(start).toBeEnabled({ wait: UI_MS });
    await start.click();
    // #256: the ride has started: the Ready sheet is gone and the key row is the ride's.
    await expect($(sel.readySheet("ready"))).not.toExist({ wait: UI_MS });
    await textUntil(
      sel.stateLine,
      (t) => t === "Riding",
      "the ride never says Riding",
    );
  });

  it("3. soft start: the target ramps to the plan rather than jumping", async () => {
    // #256 / #250: "ride-start soft start". TARGET is what the engine commands (Ride.tsx `commandedView`). The engine
    // holds ERG 0 W until the first pedalling sample, then ramps from the 3 s power to the block target over 5 s
    // (ADR-0002 §7.5).
    await textUntil(
      sel.slot("target"),
      (t) => numeric(t) === PLAN_W[0],
      `TARGET never reached the warm-up's ${PLAN_W[0]} W`,
      ENGAGE_MS,
    );
    const targets = runs(await recorded(), "target")
      .map(numeric)
      .filter((v) => v !== null);
    const reached = targets.indexOf(PLAN_W[0]);
    // The ramp: the values after the last 0 W hold, up to the first that is the plan's target.
    const hold = targets.lastIndexOf(0, reached);
    const ramp = targets.slice(hold + 1, reached + 1);
    // A jump would be one value: the target alone. A ramp has steps before it, each closer to the target.
    expect(ramp.length).toBeGreaterThanOrEqual(3);
    for (let i = 1; i < ramp.length; i++)
      expect(Math.abs(ramp[i] - PLAN_W[0])).toBeLessThan(
        Math.abs(ramp[i - 1] - PLAN_W[0]),
      );
  });

  it("4. in ERG the target is the plan's; the tiles and the chart update", async () => {
    // #256: "ride it in ERG": TARGET is the plan's target for the block, in block 1 of 3.
    await expect($(sel.slot("plan"))).toHaveText(`Plan ${PLAN_W[0]} W`);
    await expect($(sel.slot("target"))).toHaveText(String(PLAN_W[0]));
    await expect($(sel.slot("block"))).toHaveText(/· block 1 of 3$/);

    // #256: "live tiles": power, cadence and HR each show a number and change.
    const from = (await recorded()).length;
    await browser.waitUntil(
      async () => {
        const fresh = (await recorded()).slice(from);
        return ["power", "cadence", "hr"].every(
          (s) =>
            runs(fresh, s)
              .map(numeric)
              .filter((v) => v !== null).length >= 2,
        );
      },
      {
        timeout: UI_MS,
        timeoutMsg:
          "a tile (power, cadence or HR) did not show two different numbers",
      },
    );

    // #256: "a chart": each trace's line grows (Trace.tsx draws one point per second of the workout clock).
    const points = async (series) => {
      const lines = await $$(sel.chart(series));
      let n = 0;
      for (const l of lines)
        n += ((await l.getAttribute("points")) ?? "")
          .split(" ")
          .filter(Boolean).length;
      return n;
    };
    const before = {
      power: await points("power"),
      hr: await points("hr"),
      cadence: await points("cadence"),
    };
    for (const series of ["power", "hr", "cadence"])
      await browser.waitUntil(
        async () => (await points(series)) > before[series],
        {
          timeout: UI_MS,
          timeoutMsg: `the ${series} chart did not grow`,
        },
      );
  });

  it("5. Pause releases the target as designed; Resume brings it back", async () => {
    // #256: "pause". A rider pause sends the trainer to Resistance 0 % (ADR-0002 §7.9), so TARGET says "Paused"
    // (Ride.tsx `commandedView`), the band shows the Paused banner and the countdown dims.
    await $(sel.key("Pause")).click();
    await textUntil(
      sel.stateLine,
      (t) => t === "Paused",
      "the state line never said Paused",
    );
    await expect($(sel.banner("paused"))).toBeDisplayed();
    await textUntil(
      sel.slot("target"),
      (t) => t === "Paused",
      "TARGET never said Paused",
    );
    await expect($(`${sel.slot("countdown")}[data-dim]`)).toExist();
    await expect($(sel.key("Pause"))).toBeDisabled();

    // #256: "pause" (resume half). Resume re-engages ERG through the soft start: Riding, and TARGET back on the plan.
    await $(sel.key("Resume")).click();
    await textUntil(
      sel.stateLine,
      (t) => t === "Riding",
      "the state line never said Riding again",
    );
    await expect($(sel.banner("paused"))).not.toExist();
    await textUntil(
      sel.slot("target"),
      (t) => numeric(t) === PLAN_W[0],
      `TARGET did not come back to ${PLAN_W[0]} W after Resume`,
      ENGAGE_MS,
    );
  });

  it("6. Skip moves to the next block", async () => {
    // #256: "skip". The ride is still in its first block (well under its minute), so Skip moves to block 2, whose
    // plan target is 200 W, and the engine commands it.
    await expect($(sel.slot("block"))).toHaveText(/· block 1 of 3$/);
    await $(sel.key("Skip block")).click();
    await textUntil(
      sel.slot("block"),
      (t) => /· block 2 of 3$/.test(t),
      "Skip did not move to block 2",
    );
    await expect($(sel.slot("plan"))).toHaveText(`Plan ${PLAN_W[1]} W`);
    await textUntil(
      sel.slot("target"),
      (t) => numeric(t) === PLAN_W[1],
      `TARGET did not follow block 2's ${PLAN_W[1]} W`,
      ENGAGE_MS,
    );
  });

  it("7. a trainer dropout, then a strap dropout: each shows its loss, then the reconnect", async () => {
    // #256: "a sim dropout of the trainer […] shows the loss state, then the reconnect". The trainer's banner says it
    // is lost (banners.tsx, DESIGN §6.5). The pill is not checked here: between attempts it reads "lost" or
    // "connecting".
    expect(await simFault("trainer", DOWN_MS)).toBeNull();
    await expect($(sel.banner("loss-trainer"))).toBeDisplayed({ wait: UI_MS });
    // The reconnect: the banner goes, the pill is ready again, and the status bar's chip names the gap for 10 s
    // (Ride.tsx `BACK_CHIP_S`). The gap is at least the loss's length.
    await expect($(sel.banner("loss-trainer"))).not.toExist({ wait: UI_MS });
    const back = await textUntil(
      sel.neoBack,
      (t) => /^NEO back · \d+:\d\d gap$/.test(t),
      'the "NEO back · m:ss gap" chip never showed after the trainer came back',
    );
    const [m, ss] = back
      .match(/(\d+):(\d\d) gap$/)
      .slice(1)
      .map(Number);
    expect(m * 60 + ss).toBeGreaterThanOrEqual(DOWN_MS / 1000);
    await expect($(sel.pill("NEO"))).toHaveAttribute("data-link", "ready", {
      wait: UI_MS,
    });
    // The ride goes on after the trainer is back: Riding, and the soft start brings TARGET back to the plan (block 2's,
    // or block 3's if block 2 ran out meanwhile, so it is read from the plan slot).
    await textUntil(
      sel.stateLine,
      (t) => t === "Riding",
      "the ride did not say Riding after the trainer came back",
      ENGAGE_MS,
    );
    let plan = null;
    await browser.waitUntil(
      async () => {
        plan = Number(
          (await text(sel.slot("plan"))).match(/^Plan (\d+) W$/)?.[1],
        );
        return numeric(await text(sel.slot("target"))) === plan;
      },
      {
        timeout: ENGAGE_MS,
        timeoutMsg: `TARGET did not come back to the plan (${plan} W) after the reconnect`,
      },
    );

    // #256: "[…] and the strap shows the loss state, then the reconnect". HR lost: its banner, and the HR tile shows —
    // (Ride.tsx `Value`); the ride goes on.
    expect(await simFault("heart_rate", DOWN_MS)).toBeNull();
    await expect($(sel.banner("loss-hr"))).toBeDisplayed({ wait: UI_MS });
    await expect($(sel.slot("hr"))).toHaveText("—");
    await expect($(sel.banner("loss-trainer"))).not.toExist();
    // The reconnect: the banner goes, the pill is ready, and the HR tile shows a number again.
    await expect($(sel.banner("loss-hr"))).not.toExist({ wait: UI_MS });
    await expect($(sel.pill("HR"))).toHaveAttribute("data-link", "ready", {
      wait: UI_MS,
    });
    await textUntil(
      sel.slot("hr"),
      (t) => numeric(t) !== null,
      "the HR tile did not show a number after the strap came back",
    );
    await expect($(sel.stateLine)).toHaveText("Riding");
  });

  it("8. End → the summary shows the ride's basics", async () => {
    // #256: "end". End asks first; End ride ends it, and the app lands on the ride's summary (History, #106 MVP).
    await $(sel.key("End")).click();
    await $(sel.confirmEnd).click();
    await expect($(sel.summary)).toBeDisplayed({ wait: UI_MS });
    // #256: "the ride is saved, with a basic summary": its name, a finished outcome (an orderly End is not forced),
    // a duration and the average power and heart rate the core recorded.
    await expect($(sel.heading)).toHaveText(WORKOUT);
    await expect($(sel.outcome)).toHaveAttribute("data-outcome", "finished");
    await expect($(sel.outcome)).toHaveText("Finished");
    const duration = await text(sel.stat("Duration"));
    expect(duration).toMatch(/^\d+:\d\d$/);
    expect(duration).not.toBe("0:00");
    await expect($(sel.stat("Average power"))).toHaveText(/^\d+ W$/);
    await expect($(sel.stat("Average heart rate"))).toHaveText(/^\d+ bpm$/);

    // For step 9: what the relaunched app must read back from the data dir.
    mkdirSync(outDir, { recursive: true });
    writeFileSync(
      join(outDir, "mvp-ride.json"),
      JSON.stringify({ workout: WORKOUT, duration }),
    );
  });

  it("quits the app by closing its window, so step 9 relaunches it", async () => {
    // Closing the last window is an exit request (app/src-tauri/src/lifecycle.rs). Wait until the process is gone,
    // so the relaunch never meets this one's `ride.lock`. The bound is far above the 3 s quit cap.
    const app = process.env.CRANK_E2E_APP;
    expect(app).toBeTruthy();
    await browser.closeWindow();
    const running = () => {
      try {
        execFileSync("pgrep", ["-f", app], { stdio: "ignore" });
        return true;
      } catch {
        return false;
      }
    };
    const deadline = Date.now() + UI_MS;
    while (running()) {
      if (Date.now() > deadline)
        throw new Error(
          `the app (${app}) was still running ${UI_MS} ms after its window closed`,
        );
      await new Promise((r) => setTimeout(r, 100));
    }
  });
});
