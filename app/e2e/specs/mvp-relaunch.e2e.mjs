// The MVP walk, step 9 (#667): the ride is in History after a relaunch, read back from the data dir.
// mvp.e2e.mjs ends by closing the window, which quits the app. Before this spec, the service finds the embedded
// WebDriver gone and starts the app again with the same environment, so on the same `CRANK_DATA_DIR`
// (@wdio/tauri-service `ensureEmbeddedServersHealthy`). Nothing of the first process is left: the ride session and the
// route were in its webview, so this History reads only what `rides_list` and `ride_summary` return from `crank.db`.
// wdio.conf.mjs runs this spec right after mvp.e2e.mjs.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { $, $$, expect } from "@wdio/globals";
import * as sel from "./mvp-selectors.mjs";

const outDir = process.env.CRANK_E2E_OUT ?? "e2e-out";
const UI_MS = 10_000;

describe("MVP walk, after a relaunch (#256 critical path, sim)", () => {
  it("9. the ride is in History, read back from the data dir", async () => {
    const rode = JSON.parse(
      readFileSync(join(outDir, "mvp-ride.json"), "utf8"),
    );
    // A new process: the shell starts on Today, not on the summary the last one showed (the route is not persisted,
    // src/router.ts).
    await expect($(sel.heading)).toHaveText("Today", { wait: UI_MS });

    // #256: "the ride is saved": History lists exactly the one ride of this scratch data dir, finished.
    await $(sel.rail("History")).click();
    await expect($(sel.heading)).toHaveText("History");
    await expect($$(sel.historyRows)).toBeElementsArrayOfSize(1, {
      wait: UI_MS,
    });
    const review = $(sel.reviewButton(rode.workout));
    await expect(review).toBeDisplayed();
    // The Review button's name is the row: name, start, outcome, duration (History.tsx `RideRow`).
    await expect(review).toHaveAttribute(
      "aria-label",
      new RegExp(`, Finished, ${rode.duration}$`),
    );

    // #256: "with a basic summary": the stored summary has the same basics the first process showed after End.
    await review.click();
    await expect($(sel.summary)).toBeDisplayed({ wait: UI_MS });
    await expect($(sel.heading)).toHaveText(rode.workout);
    await expect($(sel.outcome)).toHaveAttribute("data-outcome", "finished");
    await expect($(sel.stat("Duration"))).toHaveText(rode.duration);
    await expect($(sel.stat("Average power"))).toHaveText(/^\d+ W$/);
    await expect($(sel.stat("Average heart rate"))).toHaveText(/^\d+ bpm$/);
  });
});
