// The MVP walk (#667): the whole #256 critical path driven through the real app's UI in sim mode, asserting what the
// rider sees. The steps follow #667's list; each assertion names its #256 step ("#256: …").
// Only DOM and visible text are asserted (selectors in mvp-selectors.mjs, each with its component), and no test probe
// is read: #656's `window.__crankUi` is not used. The one helper this file puts in the page (`record`) only reads the
// ride screen's slots, so a soft start's ramp and the live tiles can be seen between two WebDriver calls; the other
// (`watch`) records one attribute of one node on every DOM change, so a link phase that lasts a second is seen however
// the WebDriver calls fall. The `invoke`s are causes, the e2e build's test hooks (app/src-tauri/src/commands.rs): step
// 7's `sim_fault` (#668) makes the simulated trainer or strap drop out in the real core, and `sim_latency` (#681) makes
// their connects take long enough for the link chips' phases to be seen (the sim connects at once otherwise), or hides
// the trainer. What the rider then sees is read from the DOM like the rest.
//
// The chips are driven by Ride this, whose load connects the devices whatever the window's intent (ADR-0002 §5.1.4),
// not by the warm views' `Likely`: a window the run cannot bring to the front (a locked session, #687) is `hidden` to
// its page, which then tells the keeper `Release(Hidden)`. Retry's effect (a `Likely` too) is therefore the unit
// tests' (links/chips.test.tsx, links/intent.test.ts); the walk checks that it is offered. No view with link chips is
// reachable during a ride (the ride screen has no rail): step 7's losses are read from the status bar's device pill,
// and step 8 checks that the chips come back to the keeper's phases once the ride has ended.
//
// The ride is a workout the walk builds and saves first (step 0, #110, #653): New workout, then the built-in "Sensor
// check (3 min)" (crates/crank-core/src/library.rs `BUILTIN`) step by step from the outline (Add step, the step's form,
// Apply; a Duplicate and a Delete on the way), which the real core prints as the text; a name, Save. It is ridden
// through the library's Ride
// this by its id (#643): 1 min each at 50 %, 80 % and 40 % of FTP. A fresh store's FTP is its prior, 250 W (ADR-0009
// §2.1.2); step 0b types in 300 W in Settings → Athlete model (#716), and the load compiles the workout at the model's
// FTP (ADR-0009 §2.1.4), so its plan targets are 150 W, 240 W and 120 W, not the prior's 125, 200 and 100. The simulator's scan offers "crank sim trainer"
// and "crank sim strap" (crates/crank-core/src/backend.rs `Backend::scan`), and its rider pedals on its own.
//
// Step 9 (the ride is in History after a relaunch) is mvp-relaunch.e2e.mjs. This spec ends by closing the window,
// which quits the app (AGENTS.md "Quitting ends a ride"). The service starts it again on the same data dir for the
// next spec (wdio.conf.mjs runs the specs in order).
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { browser, $, $$, expect } from "@wdio/globals";
import { Key } from "webdriverio";
import * as sel from "./mvp-selectors.mjs";

const outDir = process.env.CRANK_E2E_OUT ?? "e2e-out";
const WORKOUT = "My sensor check";
/** Its steps, as the outline's form takes them: the built-in sensor check's (library.rs `BUILTIN`). */
const WORKOUT_STEPS = [
  ["Warm-up", "50"],
  ["Work", "80"],
  ["Cool-down", "40"],
];
/** Its text as the core prints it (docs/DSL.md "Canonical printing": one top-level step per line). */
const WORKOUT_DSL = '"Warm-up" 1m 50%\n"Work" 1m 80%\n"Cool-down" 1m 40%';
/** A step step 0 types at the end of the text, then undoes (#759). */
const TYPED_STEP = '"Spin" 1m 30%';
/** The FTP step 0b types in, W: not the prior's 250 W, so the ride's targets show which one they were set from. */
const FTP_W = 300;
/** The plan's target per block, W (50 %, 80 %, 40 % of the FTP typed in). */
const PLAN_W = [0.5, 0.8, 0.4].map((f) => Math.round(f * FTP_W));
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
 * The e2e build's test hook (#681): from the simulated trainer's (`"trainer"`) or strap's (`"heart_rate"`) next connect
 * on, finding it takes `resolveMs`, its GATT connect `gattMs` and the trainer's handshake `handshakeMs`; with
 * `advertising` false it is not found (crank-core `Core::sim_latency`). Resolves to `null`, or to its refusal.
 */
const simLatency = (role, resolveMs, gattMs, handshakeMs, advertising = true) =>
  browser.executeAsync(
    (role, resolveMs, gattMs, handshakeMs, advertising, done) => {
      window.__TAURI_INTERNALS__
        .invoke("sim_latency", {
          role,
          resolveMs,
          gattMs,
          handshakeMs,
          advertising,
        })
        .then(() => done(null), done);
    },
    role,
    resolveMs,
    gattMs,
    handshakeMs,
    advertising,
  );
/** Each phase of a slow connect, ms: long enough to be on screen for several frames (at most 10 Hz). */
const PHASE_MS = 1_500;

/**
 * Starts recording `attr` of the node `selector` names (CSS, or XPath when it starts with `//`, as in
 * mvp-selectors.mjs) into `window.__mvpWatch[name]`: its value now (`null` while there is no such node), then each
 * change, on every DOM change of the page (a MutationObserver on the body).
 */
function watch(name, selector, attr) {
  return browser.execute(
    (name, selector, attr) => {
      const node = () =>
        selector.startsWith("//")
          ? document.evaluate(
              selector,
              document,
              null,
              XPathResult.FIRST_ORDERED_NODE_TYPE,
              null,
            ).singleNodeValue
          : document.querySelector(selector);
      const read = () => node()?.getAttribute(attr) ?? null;
      const seen = [read()];
      new MutationObserver(() => {
        const now = read();
        if (now !== seen.at(-1)) seen.push(now);
      }).observe(document.body, {
        subtree: true,
        childList: true,
        attributes: true,
        characterData: true,
      });
      window.__mvpWatch = { ...window.__mvpWatch, [name]: seen };
    },
    name,
    selector,
    attr,
  );
}
/** What `watch(name, …)` has recorded so far, `null`s (no node) left out. */
const watched = async (name) =>
  (await browser.execute((name) => window.__mvpWatch[name], name)).filter(
    (v) => v !== null,
  );
/** `seq` holds each of `want` in this order (other values may come between). */
function inOrder(seq, want) {
  let at = 0;
  for (const v of seq) if (v === want[at]) at += 1;
  return at === want.length;
}

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
  it("0. builds a workout from the outline: New workout, its steps and name, Save; it is in the library", async () => {
    // #110: the library's save loop through the real core (`workout_new`, `print_dsl`, `parse_dsl`, `compile_workout`,
    // `workout_save`), before any device is paired: the editor needs none. #653: the steps are built from the outline.
    await $(sel.rail("Workouts")).click();
    await $(sel.newWorkout).click();
    await expect($(sel.editorChip)).toHaveText("Not saved");
    // An empty workout has an error (the core's "Workout is empty"): nothing to save or ride yet.
    await expect($(sel.editorButton("Save"))).toBeDisabled({ wait: UI_MS });
    await expect($(sel.editorButton("Ride this"))).toBeDisabled();
    /** Waits until the outline has `n` rows. */
    const rows = (n, what) =>
      browser.waitUntil(async () => (await $$(sel.outlineRows).length) === n, {
        timeout: UI_MS,
        timeoutMsg: `${what}: the outline never had ${n} rows`,
      });
    /** Sets a box of the selected node's form. */
    const set = async (label, value) => {
      await $(sel.blockField(label)).clearValue();
      await $(sel.blockField(label)).setValue(value);
    };
    for (const [i, [label, power]] of WORKOUT_STEPS.entries()) {
      // Add step inserts a 5 min step at 50 % after the selected one (the first: at the end), selected and focused.
      await $(sel.outlineTool("Add step")).click();
      await rows(i + 1, `Add step ${i + 1}`);
      await expect($(sel.outlineSelected)).toBeFocused();
      await expect($(sel.outlineSelected)).toHaveText("5:00 · 50 %");
      await set("Label", label);
      await set("Minutes", "1");
      await set("Power", power);
      await $(sel.blockApply).click();
      await expect($(sel.outlineSelected)).toHaveText(
        `${label} · 1:00 · ${power} %`,
        { wait: UI_MS },
      );
    }
    // A Duplicate, then a Delete of the copy: back to the three steps.
    await $(sel.outlineTool("Duplicate")).click();
    await rows(4, "Duplicate");
    await $(sel.outlineTool("Delete")).click();
    await rows(3, "Delete");
    // The real core printed every step: the text view holds the canonical text.
    await expect($(sel.editorText)).toHaveText(WORKOUT_DSL, { wait: UI_MS });
    // #759, ADR-0011 §2.2: typing in the text box (CodeMirror in WKWebView) is read by the real core, one burst is one
    // undo step, and Cmd+Z in the box undoes it in the text and the outline alike (the model's history, not the DOM's).
    await $(sel.editorText).click();
    await browser.keys([Key.Command, Key.ArrowDown]);
    await browser.keys([Key.Enter, ...TYPED_STEP]);
    await rows(4, "a step typed in the text");
    await expect($(sel.editorText)).toHaveText(`${WORKOUT_DSL}\n${TYPED_STEP}`);
    await browser.keys([Key.Command, "z"]);
    await rows(3, "Cmd+Z after typing");
    await expect($(sel.editorText)).toHaveText(WORKOUT_DSL);
    await $(sel.editorName).setValue(WORKOUT);
    await expect($(sel.editorText)).toHaveText(WORKOUT_DSL);
    await expect($(sel.editorName)).toHaveValue(WORKOUT);
    await expect($(sel.editorButton("Save"))).toBeEnabled({ wait: UI_MS });
    await $(sel.editorButton("Save")).click();
    await expect($(sel.editorChip)).toHaveText("Saved", { wait: UI_MS });
    // Saved: Ride this rides it by its id (step 2 rides it from the library, as a rider who saved it earlier would).
    await expect($(sel.editorButton("Ride this"))).toBeEnabled();
    await $(sel.backToLibrary).click();
    await expect($(sel.workoutOption(WORKOUT))).toBeDisplayed({ wait: UI_MS });
    // The built-in it copies is unchanged beside it.
    await expect($(sel.workoutOption("Sensor check (3 min)"))).toBeDisplayed();
  });

  it("0b. types in an FTP in Settings → Athlete model: the model shows it", async () => {
    // #716, ADR-0009 §2.1.3: a fresh store's FTP is the prior, said as an estimate; Edit, the value, Save. Steps 3 to 6
    // check that the ride's targets are set from it (§2.1.4).
    await $(sel.rail("Settings")).click();
    await $(sel.settingsSection("Athlete model")).click();
    await expect($(sel.athleteValue("ftp"))).toHaveText(
      "estimate: 250 W (no test yet)",
      { wait: UI_MS },
    );
    await $(sel.athleteEdit("FTP")).click();
    await $(sel.athleteField("FTP, W")).setValue(String(FTP_W));
    await $(sel.athleteSave).click();
    // The core's default ± for a typed-in value is 20 % (athlete/prior.rs `SelfReportOrigin::Guess`).
    await expect($(sel.athleteValue("ftp"))).toHaveText(
      `${FTP_W} W ± ${FTP_W * 0.2}`,
      { wait: UI_MS },
    );
  });

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

  it("1b. a trainer that is not found: the load fails, and its chip says so, with the hint and Retry", async () => {
    // #681, ADR-0002 §5.1.4 "Failure UX". Settings → Devices released the links for pairing (`Release(Devices)`): each
    // paired row's chip says so.
    for (const role of ["trainer", "strap"])
      await expect($(sel.linkChip(role))).toHaveAttribute(
        "data-state",
        "cold",
        { wait: UI_MS },
      );
    // A trainer that does not advertise (asleep, or held by another app). Ride this connects it (the load's own
    // attempt, §5.1.4, whatever the window's intent), and the attempt ends not found: the load is refused, and the
    // chip under Ride this says why, with the hedged hint and Retry.
    expect(
      await simLatency("trainer", PHASE_MS, PHASE_MS, PHASE_MS, false),
    ).toBeNull();
    await $(sel.rail("Workouts")).click();
    await $(sel.workoutOption(WORKOUT)).click();
    await watch("refused", sel.linkChip("trainer"), "data-state");
    await $(sel.rideThis).click();
    await expect($(sel.linkChip("trainer"))).toHaveAttribute(
      "data-state",
      "not-found",
      { wait: UI_MS },
    );
    expect(inOrder(await watched("refused"), ["searching", "not-found"])).toBe(
      true,
    );
    await expect($(sel.linkChip("trainer"))).toHaveText("Trainer ⚠ not found");
    await expect(
      $(sel.linkHint("Trainer: is another app using it?")),
    ).toBeDisplayed();
    await expect($(sel.linkRetry)).toBeDisplayed();
    // The strap is not the trainer's failure: its attempt beside it linked.
    await expect($(sel.linkChip("strap"))).toHaveAttribute(
      "data-state",
      "ready",
      { wait: UI_MS },
    );
  });

  it("2. picks a workout, Ride this, the Ready sheet, Start", async () => {
    // #681: the trainer advertises again, slow to connect. Ride this connects it, and the chip under it shows the
    // connect's phases (`searching`, then `connecting`, the handshake included) until the ride holds the link
    // (ADR-0002 §5.1.4).
    await watch("workouts", sel.linkChip("trainer"), "data-state");
    expect(
      await simLatency("trainer", PHASE_MS, PHASE_MS, PHASE_MS),
    ).toBeNull();
    // #256: "pick a workout" (Workouts → the row → Ride this) loads the ride and opens the ride screen. The workout is
    // still picked from step 1b.
    await expect($(sel.linkRetry)).toBeDisplayed();
    // The refused load is over before Ride this is pressed again: the button is enabled once the refusal has come back
    // and the frame no longer says `loading` (Workouts.tsx). The chip above can say "not found" a frame earlier.
    await expect($(sel.rideThis)).toBeEnabled({ wait: UI_MS });
    await $(sel.rideThis).click();
    // #256: both devices connected for the ride. The Ready sheet's rows read the frame's devices: the ride holds both
    // (`riding`) on ready links (ReadySheet.tsx `phase`); the status bar's pill shows the same links (Ride.tsx `Pill`).
    await expect($(sel.readySheet("ready"))).toBeDisplayed({ wait: UI_MS });
    // The Workouts chip went through the trainer's connect before the ride took the link.
    const chip = await watched("workouts");
    expect(inOrder(chip, ["searching", "connecting"])).toBe(true);
    // An instant connect again, so step 7's reconnect takes the sim's usual time.
    expect(await simLatency("trainer", 0, 0, 0)).toBeNull();
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
    // plan target is 240 W (80 % of the FTP typed in), and the engine commands it.
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
    // is lost (banners.tsx, DESIGN §6.5), and so does the status bar's device pill (Ride.tsx `Pill`, not a link chip:
    // the ride screen has no rail, so no view with link chips is reachable during the ride). Between attempts the pill
    // reads "lost" or "connecting", so its values are recorded and the loss must be among them. The link chips are
    // checked after the ride, in step 8.
    await watch("neo-pill", sel.pill("NEO"), "data-link");
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
    expect(inOrder(await watched("neo-pill"), ["ready", "lost", "ready"])).toBe(
      true,
    );
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
    await watch("hr-pill", sel.pill("HR"), "data-link");
    expect(await simFault("heart_rate", DOWN_MS)).toBeNull();
    await expect($(sel.banner("loss-hr"))).toBeDisplayed({ wait: UI_MS });
    await expect($(sel.slot("hr"))).toHaveText("—");
    await expect($(sel.banner("loss-trainer"))).not.toExist();
    // The reconnect: the banner goes, the pill is ready, and the HR tile shows a number again.
    await expect($(sel.banner("loss-hr"))).not.toExist({ wait: UI_MS });
    await expect($(sel.pill("HR"))).toHaveAttribute("data-link", "ready", {
      wait: UI_MS,
    });
    expect(inOrder(await watched("hr-pill"), ["ready", "lost", "ready"])).toBe(
      true,
    );
    await textUntil(
      sel.slot("hr"),
      (t) => numeric(t) !== null,
      "the HR tile did not show a number after the strap came back",
    );
    await expect($(sel.stateLine)).toHaveText("Riding");
  });

  it("8. End → the summary shows the ride's basics; the link chips are the keeper's again", async () => {
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

    // #681: after the ride the devices are the keeper's again. The ride's end released both (`cold`, ADR-0002 §5.1.1),
    // so Today's link chips show the keeper's phase, never the ride's stale "connected" nor step 7's "lost". (On a
    // visible window the keeper then warms them up; on this host's locked session the page is hidden, #687.)
    await $(sel.rail("Today")).click();
    const KEEPER = ["cold", "searching", "connecting", "ready"];
    for (const role of ["trainer", "strap"]) {
      let state = null;
      await browser.waitUntil(
        async () => {
          state = await $(sel.linkChip(role)).getAttribute("data-state");
          return KEEPER.includes(state);
        },
        {
          timeout: UI_MS,
          timeoutMsg: `the ${role} chip did not come back to the keeper after the ride (last: ${state})`,
        },
      );
      await expect($(sel.linkChip(role))).not.toHaveAttribute(
        "data-phase",
        "riding",
      );
    }
  });

  it("quits the app by closing its window, so step 9 relaunches it", async () => {
    // Closing the last window is an exit request (app/src-tauri/src/lifecycle.rs). Wait until the process is gone,
    // so the relaunch never meets this one's `ride.lock`. The bound is far above the 3 s quit cap.
    const app = process.env.CRANK_E2E_APP;
    expect(app).toBeTruthy();
    // The embedded WebDriver answers Close Window before the window is gone (tauri-plugin-wdio-webdriver 1.5.0
    // `close_window`: `destroy()` only queues it), so the handles it returns still name it. WebdriverIO 9 then switches
    // to the first of them on its own (its ContextManager, on closeWindow's result), a request nobody awaits that meets
    // the quitting app. Wait for that request to settle, so it never outlives this test (#671).
    const followUp = new Promise((resolve) => {
      const onResult = (event) => {
        if (event.command !== "switchToWindow") return;
        browser.off("result", onResult);
        resolve(event.result);
      };
      browser.on("result", onResult);
    });
    const handles = await browser.closeWindow();
    expect(handles).toHaveLength(1);
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
    // That switch was sent to a window being destroyed on an app that is exiting: it cannot have succeeded.
    expect((await followUp).error).toBeInstanceOf(Error);
    // The WebDriver session ended with the app's process, just asserted gone: nothing is left to delete. Without this,
    // the runner's teardown sends DELETE /session to the dead server, which throws outside the framework, so the
    // worker fails and its reporter output (steps 1–8 above) is lost (#671). In @wdio/runner 9.31.9 (build/index.js):
    // `run` calls `endSession()` after the framework, outside its try/catch (:811); `endSession` (:959) computes
    // `hasSessionId` from `this._browser.sessionId` (:962) and returns before any request when it is unset and no
    // session is passed to attach to (:970); otherwise it awaits `deleteSession()` (:980), and after that DELETE it
    // clears `sessionId` itself (:991). wdio has no public option for "this session is already gone"; an unset
    // `sessionId` is the runner's own mark for an ended session, the one state in which `endSession` skips the DELETE.
    // It must be set on the runner's instance: `browser` from @wdio/globals is a proxy with only a `get` trap
    // (@wdio/globals 9.31.3 build/index.js:6), so an assignment through it never reaches the runner, and the runner puts
    // that instance on `globalThis.browser` (@wdio/runner :879 → @wdio/globals `_setGlobal`, :94–97; `injectGlobals`
    // is on by default). The assertions check it is the same session, and that the runner's instance now has none.
    const session = globalThis.browser;
    expect(session?.sessionId).toBe(browser.sessionId);
    session.sessionId = undefined;
    expect(browser.sessionId).toBeUndefined();
  });
});
