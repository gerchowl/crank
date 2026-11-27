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

// Settings → Athlete model (src/views/Athlete.tsx `Athlete`, ADR-0009 §2.1.3).
/** A parameter's value ("estimate: 250 W (no test yet)", "300 W ± 60", "Not set"), by its stored name ("ftp"). */
export const athleteValue = (param) =>
  `[data-testid=${lit(`athlete-${param}`)}]`;
/** A parameter row's Edit, by the parameter's label ("FTP"); its name is "Edit FTP". */
export const athleteEdit = (label) =>
  `button[aria-label=${lit(`Edit ${label}`)}]`;
/** The open draft's value box, by its name ("FTP, W"). */
export const athleteField = (name) =>
  `form.athlete-edit input[aria-label=${lit(name)}]`;
/** The open draft's Save. */
export const athleteSave = `//form[@class="athlete-edit"]//button[normalize-space(.)="Save"]`;

// The link chips (src/links/chips.tsx `LinkChip`, `RetryLinks`) on Today, Workouts and Settings → Devices.
/**
 * A device's chip ("trainer", "strap"): `data-state` is what it says ("cold", "searching", "connecting", "ready",
 * "not-found", "connected", "lost"), `data-phase` the keeper's phase. One per device on each of those views.
 */
export const linkChip = (role) => `[data-link-chip=${lit(role)}]`;
/** The Retry of a warm view, shown while a device is not found. */
export const linkRetry = `button.link-retry`;
/** The hint line of a device that is not found ("Trainer: is another app using it?"). */
export const linkHint = (text) =>
  `//p[@class="link-hint" and normalize-space(.)=${lit(text)}]`;

// Workouts (src/views/Workouts.tsx `Workouts`).
/** A library row, by the workout's name. */
export const workoutOption = (name) =>
  `//ul[@role="listbox" and @aria-label="Library"]/li[@role="option"][span[@class="workouts-name" and normalize-space(.)=${lit(name)}]]`;
/** The picked workout's Ride this. */
export const rideThis = `//div[@class="workouts-buttons"]/button[normalize-space(.)="Ride this"]`;
/** New workout, beside the page title. */
export const newWorkout = `//div[@class="workouts-head"]/button[normalize-space(.)="New workout"]`;

// The workout editor (src/editor/WorkoutEditor.tsx `WorkoutEditor`, #110).
/** The workout's name. */
export const editorName = `//label[@class="editor-name"]/input`;
/** The DSL text: CodeMirror's content (src/editor/TextView.tsx, ADR-0011 §2.1), a multi-line text box. */
export const editorText = `//div[@id="editor-dsl" and @role="textbox"]`;
/** A button of the saving row ("Save", "Save as new", "Ride this"). */
export const editorButton = (word) =>
  `//div[@class="editor-store"]//button[normalize-space(.)=${lit(word)}]`;
/** The chip saying where the workout stands ("Not saved", "Unsaved changes", "Saved"). */
export const editorChip = `//div[@class="editor-title"]/span[@class="workouts-chip"]`;
// Outline editing (src/editor/OutlineEdit.tsx `OutlineToolbar`, `BlockForm`, #653).
/** A button of the outline's toolbar ("Add step", "Duplicate", "Wrap in repeat", "Unwrap", "Move up", "Move down", "Delete"). */
export const outlineTool = (word) =>
  `//div[@role="toolbar" and @aria-label="Outline actions"]/button[normalize-space(.)=${lit(word)}]`;
/** The outline's rows. */
export const outlineRows = `//ul[@id="editor-outline"]/li[@role="option"]`;
/** The outline's selected row. */
export const outlineSelected = `//ul[@id="editor-outline"]/li[@role="option" and @aria-selected="true"]`;
/** A box of the selected node's form, by its label ("Label", "Minutes", "Seconds", "Power", "Count", …). */
export const blockField = (label) =>
  `//form[@class="editor-block"]//div[@class="editor-field"][label[normalize-space(.)=${lit(label)}]]/input`;
/** The form's Apply. */
export const blockApply = `//form[@class="editor-block"]//button[normalize-space(.)="Apply"]`;
/** Back to library. */
export const backToLibrary = `//div[@class="editor-head"]//button[normalize-space(.)="Back to library"]`;

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
/**
 * A device's half of the status bar's pill ("NEO", "HR") (src/ride/Ride.tsx `Pill`): `data-link` is the link the core
 * reported ("ready", "connecting", "lost"), or "none" while none is reported. A lost link's pill starts with "⚠ ".
 */
export const pill = (name) =>
  `//div[@aria-label="Devices"]/span[contains(concat(" ", @class, " "), " pill-half ") and (starts-with(normalize-space(.), ${lit(`${name} `)}) or starts-with(normalize-space(.), ${lit(`⚠ ${name} `)}))]`;
/** A key-row button by its word ("Start", "Pause", "Resume", "Skip block", "End"); the key hint follows the word. */
export const key = (word) =>
  `//div[@data-region="keys"]/button[normalize-space(.)=${lit(word)} or starts-with(normalize-space(.), ${lit(`${word} `)})]`;
/** A live slot: "target", "plan", "power", "cadence", "hr", "block", "countdown", "elapsed". */
export const slot = (name) => `[data-slot=${lit(name)}]`;
/** The status bar's reconnect chip: "NEO back · m:ss gap" for a while after a trainer loss, else empty. */
export const neoBack = `[data-testid="neo-back"]`;
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
