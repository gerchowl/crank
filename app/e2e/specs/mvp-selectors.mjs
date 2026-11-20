// The selectors of the MVP walk (#667): `mvp.e2e.mjs` and `mvp-relaunch.e2e.mjs` drive the real app with them
// (WebdriverIO), and `e2e/design/mvp-selectors.pw.ts` checks each one against the design fixtures, which mount the same
// components (Chromium, in CI). Each one names the component that renders it. Only DOM and visible text: no
// test probe (#656's `window.__crankUi` is not used), so the walk runs on dev as it is.
//
// A string starting with `//` is XPath, in both WebdriverIO and Playwright; any other is CSS.

/** The text literal of an XPath expression (no value here contains both quote kinds). */
const lit = (s) => (s.includes('"') ? `'${s}'` : `"${s}"`);

/** A rail place (src/shell/Rail.tsx `Rail`): the button over its word. */
export const rail = (place) =>
  `//nav[@aria-label="Places"]/button[.//span[@class="rail-label" and normalize-space(.)=${lit(place)}]]`;

/** The page's title (every view's `<h1>`). */
export const heading = "h1";

// Settings → Devices (src/views/Settings.tsx `Settings`, src/views/Devices.tsx `Devices`).
/** A section in the Settings list. */
export const settingsSection = (label) =>
  `//nav[@aria-label="Settings sections"]/button[normalize-space(.)=${lit(label)}]`;
/** The Nearby group's scan button ("Scan", then "Scan again"). */
export const scanButton = `//section[@aria-labelledby="scan-h"]//button[normalize-space(.)="Scan" or normalize-space(.)="Scan again"]`;
/** A nearby device's pair button; its name starts with the visible words, then names the device. */
export const pairButton = (role, name) =>
  `button[aria-label=${lit(`Pair as ${role}: ${name}`)}]`;
/** The nearby row's mark once that device holds the role. */
export const pairedMark = (role, name) =>
  `//ul[@aria-label="Nearby devices"]/li[strong[normalize-space(.)=${lit(name)}]]//span[@class="device-paired-word" and normalize-space(.)=${lit(`✓ Paired as ${role}`)}]`;
/** The Paired table's row for a role ("Trainer", "Heart-rate strap"): "Loading…", "Not paired" or the device. */
export const pairedRow = (roleLabel) =>
  `//section[@aria-labelledby="paired-h"]//li[.//strong[normalize-space(.)=${lit(roleLabel)}]]`;
/** The Paired table's line for a role ("Trainer", "Heart-rate strap"); `device-line` only while a device is paired. */
export const pairedLine = (roleLabel) =>
  `//section[@aria-labelledby="paired-h"]//li[.//strong[normalize-space(.)=${lit(roleLabel)}]]/span[@class="device-line"]`;

// Workouts (src/views/Workouts.tsx `Workouts`).
/** A library row, by the workout's name. */
export const workoutOption = (name) =>
  `//ul[@role="listbox" and @aria-label="Library"]/li[@role="option"][span[@class="workouts-name" and normalize-space(.)=${lit(name)}]]`;
/** The picked workout's Ride this. */
export const rideThis = `//div[@class="workouts-buttons"]/button[normalize-space(.)="Ride this"]`;

// The Ready sheet (src/ride/ReadySheet.tsx `ReadyBody`).
/** The sheet in a state (`ReadyState`). */
export const readySheet = (state) =>
  `section.ready-sheet[data-ready=${lit(state)}]`;
/** A device row ("trainer" or "hr") in a phase (`DevicePhase`). */
export const readyDevice = (device, phase) =>
  `li.ready-device[data-device=${lit(device)}][data-state=${lit(phase)}]`;

/** The sheet's plan line ("ERG · starts with 1:00 warm-up"). */
export const readyPlan = `[data-testid="ready-plan"]`;

// The ride screen (src/ride/Ride.tsx `Screen`).
/** The screen's root. */
export const ride = `[data-testid="ride"]`;
/** A key-row button by its word ("Start", "Pause", "Resume", "Skip block", "End"); the key hint follows the word. */
export const key = (word) =>
  `//div[@data-region="keys"]/button[normalize-space(.)=${lit(word)} or starts-with(normalize-space(.), ${lit(`${word} `)})]`;
/** A live slot: "target", "plan", "power", "cadence", "hr", "block", "countdown", "elapsed". */
export const slot = (name) => `[data-slot=${lit(name)}]`;
/** The key row's state line ("Riding", "Paused", …). */
export const stateLine = `[data-testid="state"]`;
/** A banner of the band by its id (src/ride/banners.tsx `BannerBand`): "paused", "loss-trainer", "loss-hr", … */
export const banner = (id) => `[data-testid=${lit(id)}]`;
/** A trace's line (src/ride/Trace.tsx `Trace`): "power", "hr", "cadence". */
export const chart = (series) =>
  `polyline[data-testid=${lit(`chart-${series}`)}]`;
/** The End dialog's confirming button. */
export const confirmEnd = `//div[@role="alertdialog"]/button[normalize-space(.)="End ride"]`;

// History and the ride's summary (src/views/History.tsx `RideSummaryView`, `RideList`).
/** The summary of one ride. */
export const summary = `section[aria-label="Ride summary"]`;
/** The summary's outcome (`data-outcome` is the core's `RideOutcome`). */
export const outcome = `${summary} .review-outcome`;
/** A figure of the summary, by its term ("Duration", "Average power", …). */
export const stat = (term) =>
  `//dl[@class="review-stats"]/div[dt[normalize-space(.)=${lit(term)}]]/dd`;
/** The rows of the Rides table. */
export const historyRows = `//div[@role="table" and @aria-label="Rides"]//div[@role="row" and contains(concat(" ", @class, " "), " history-row ")]`;
/** A row's Review button: its accessible name tells the ride apart (name, start, outcome, duration). */
export const reviewButton = (workoutName) =>
  `${historyRows}//button[starts-with(@aria-label, ${lit(`Review ${workoutName}, `)})]`;
