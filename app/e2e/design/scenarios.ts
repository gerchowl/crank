// The design fixtures (#457): every screen and state the MVP uses, by the name `fixture.html?scenario=<name>` takes.
// Pure data, shared by the fixture entry (fixture.tsx, which sets each one up on the repo's mockIPC backends) and the
// Playwright spec (design.pw.ts, which opens, settles and audits each one). Each fixture is a static state, not a
// simulation: nothing ticks, so two runs render the same page.

export interface Scenario {
  /** The `?scenario=` value. */
  name: string;
  /** A Playwright selector that is visible once the state is on screen. */
  ready: string;
  /** Listbox options the spec clicks, by accessible name, before its `clicks` (a workout picked in the library). */
  picks?: string[];
  /** Buttons the spec clicks, by accessible name, before it waits for `ready` (a scan, Ride this). */
  clicks?: string[];
  /** Text boxes the spec fills, by accessible name, after its `clicks` (a draft typed in the editor). */
  fills?: { label: string; text: string }[];
  /** Mock answers that stay pending on purpose (a scan in flight): the settle wait ignores this many. */
  held?: number;
  /** A selector the spec scrolls into view once the state is ready (a long list, scrolled to its end). */
  scrollTo?: string;
  /** The screen has no control by design (a slow boot's start screen): the focus walk must reach none. */
  noControls?: true;
}

/** The fixtures' wall clock (`Date`): a ride date or a "today" renders the same on every run. Timers still run. */
export const FROZEN = "2026-11-02T08:00:00Z";

export const SCENARIOS = [
  // Today (§5.1 "In direction A", #530). `today` stays first: the gate's own tests inject into it.
  { name: "today", ready: "role=list[name='Last 7 days']" },
  // First run: no rides, nothing paired; the devices say a ride needs a trainer.
  { name: "today-empty", ready: "text=No rides yet" },
  // The rides are still on the wire (StrictMode runs each mount effect twice, so the read is asked twice), and the
  // snapshot channel's first frame has not arrived: the devices are loading too.
  { name: "today-reading", ready: "text=Loading…", held: 2 },
  { name: "today-error", ready: "role=button[name='Retry']" },
  // A load started in Workouts is in flight: Start names what it is doing.
  {
    name: "today-ride-loading",
    ready: "role=button[name='Loading ride…'][disabled=true]",
  },
  // The ride is loaded (a long name): Start ⏎ leads to the ready check; the devices show its links.
  {
    name: "today-ride-loaded",
    ready: "role=button[name='Start'][disabled=false]",
  },
  // The load was refused (no trainer reached): the ride-load notice above the page says so, Today is back to nothing.
  { name: "today-ride-refused", ready: "role=alert >> text=Could not load" },
  // The loaded ride's trainer link is lost: ⚠ and its word.
  {
    name: "today-link-lost",
    ready: "role=listitem >> text=lost",
  },
  // The link chips (ADR-0002 §5.1.4, #681): the keeper's phases on Today, a warm view.
  {
    name: "today-links-warming",
    ready: "[data-link-chip='strap'][data-state='connecting']",
  },
  { name: "today-links-not-found", ready: "role=button[name='Retry']" },
  // Workouts (§5.7, #528): the library and the picked workout, in each of their states.
  { name: "workouts-empty", ready: "text=No workouts yet" },
  { name: "workouts-list-error", ready: "role=button[name='Retry']" },
  { name: "workouts", ready: "role=option[name=/Sweet spot 3x12/]" },
  {
    name: "workouts-selected",
    ready: "role=region[name='Compiled summary']",
    picks: ["Sweet spot 3x12"],
  },
  {
    name: "workouts-links",
    ready: "role=group[name='Device links'] >> role=button[name='Retry']",
    picks: ["Sweet spot 3x12"],
  },
  {
    name: "workouts-blocked",
    ready: "#ride-blocked",
    picks: ["Sweet spot 3x12"],
  },
  {
    name: "workouts-loading",
    ready: "role=button[name='Loading ride…'][disabled=true]",
    picks: ["Sweet spot 3x12"],
    clicks: ["Ride this"],
    held: 1,
  },
  {
    name: "workouts-load-error",
    ready: "text=no trainer paired",
    picks: ["Sweet spot 3x12"],
    clicks: ["Ride this"],
  },
  {
    name: "workouts-long",
    ready: "role=region[name='Compiled summary']",
    picks: ["Over-unders"],
  },
  // The editor (§5.7, #111) on a copy of the picked workout: its three views in one row.
  {
    name: "workouts-editor",
    ready: "role=listbox[name='Outline']",
    picks: ["Sweet spot 3x12"],
    clicks: ["Duplicate & edit"],
  },
  // A draft that does not parse: the saved workout stays, the problem is under the text. The text is the core's
  // `missing-target` draft (crates/crank-core/tests/fixtures/editor.json); the mock answers nothing else.
  {
    name: "workouts-editor-invalid",
    ready: "#editor-problem",
    picks: ["Sweet spot 3x12"],
    clicks: ["Duplicate & edit"],
    fills: [
      {
        label: "Text",
        text: '"Warm-up" 10m ramp 45-70%\n3x("Sweet spot" 12m 90%, "Rest" 4m)',
      },
    ],
  },
  // The rider's own workout picked (#110): Edit and Delete beside Ride this and Duplicate & edit.
  {
    name: "workouts-mine",
    ready: "role=button[name='Delete']",
    picks: ["Easy spin"],
  },
  // Delete asks first, in a dialog with the safe answer focused.
  {
    name: "workouts-delete-confirm",
    ready: "role=dialog[name='Delete Easy spin?']",
    picks: ["Easy spin"],
    clicks: ["Delete"],
  },
  // New workout (#110): an empty draft, not saved; the reasons Save and Ride this wait are under them.
  {
    name: "workouts-editor-new",
    ready: "#editor-unridden",
    clicks: ["New workout"],
  },
  { name: "devices-empty", ready: "role=button[name='Scan']" },
  {
    name: "devices-scanning",
    ready: "role=button[name='Scanning…']",
    clicks: ["Scan"],
    held: 1,
  },
  {
    name: "devices-found",
    ready: "role=list[name='Nearby devices']",
    clicks: ["Scan"],
  },
  {
    name: "devices-pairing",
    ready:
      "role=button[name='Pairing as trainer: Tacx Neo 55624'][disabled=true]",
    clicks: ["Scan", "Pair as trainer: Tacx Neo 55624"],
    held: 1,
  },
  {
    name: "devices-paired",
    ready: "role=list[name='Nearby devices']",
    clicks: ["Scan"],
  },
  {
    name: "devices-forget-confirm",
    ready: "role=group[name='Forget trainer?']",
    clicks: ["Forget trainer"],
  },
  {
    name: "devices-denied",
    ready: "text=Bluetooth access is denied",
    clicks: ["Scan"],
  },
  // The Ready sheet (§5.4, #526): the loaded, unstarted ride, in each of its states.
  { name: "ready", ready: "[data-ready='ready']" },
  // A load is under way and has not installed its ride: the sheet's loading state, drawn from the frame's `loading`
  // (the held answer is the load's).
  { name: "ready-loading", ready: "[data-ready='loading']", held: 1 },
  // The same over cold devices: each row in its device's keeper phase (#681).
  {
    name: "ready-loading-progress",
    ready: "li.ready-device[data-device='trainer'][data-state='searching']",
    held: 1,
  },
  { name: "ready-connecting", ready: "[data-ready='connecting']" },
  { name: "ready-trainer-missing", ready: "[data-ready='trainer-missing']" },
  { name: "ready-strap-missing", ready: "[data-ready='strap-missing']" },
  { name: "ready-error", ready: "[data-ready='error']", clicks: ["Start"] },
  { name: "ride-soft-start", ready: "[data-testid='soft-start']" },
  {
    name: "ride-work-step",
    ready: "role=button[name='Pause'][disabled=false]",
  },
  // The HR zone beside the live HR (DESIGN §18.3 "Heart-rate zone", #586): Friel's with an LTHR, % of HR max without.
  { name: "ride-hr-zone", ready: "[data-slot='hr-zone']" },
  { name: "ride-hr-zone-max", ready: "[data-slot='hr-zone']" },
  // An hour with a 30 s block: the strip's labels shed to a leader (DESIGN §18.3 "Strip labels", #581).
  {
    name: "ride-short-block",
    ready: "role=button[name='Pause'][disabled=false]",
  },
  { name: "ride-paused", ready: "[data-testid='paused']" },
  { name: "ride-hr-lost", ready: "[data-testid='loss-hr']" },
  { name: "ride-trainer-lost", ready: "[data-testid='loss-trainer']" },
  { name: "ride-erg-released", ready: "[data-testid='loss-erg']" },
  { name: "ride-free", ready: "[data-testid='free-ride']" },
  { name: "ride-trainer-lost-long", ready: "[data-testid='loss-help']" },
  { name: "ride-engine-silent", ready: "[data-testid='engine-unresponsive']" },
  { name: "ride-paused-hr-lost", ready: "[data-testid='loss-hr']" },
  // Review (§5.5, #529): a finished ride routes to History with its id until the summary route exists (#106).
  // A stored ride opened from the list: its figures, no traces (the core returns no samples, RideTraces.tsx).
  { name: "review", ready: "role=heading[name='Sweet spot 3x12']" },
  // The ride this window just rode: a 5-minute spin and a 3 h ride at 4 Hz with 57 blocks, traces and all.
  { name: "review-short", ready: "role=img[name=/^Power over the ride/]" },
  { name: "review-long", ready: "role=img[name=/^Power over the ride/]" },
  // Aborted, ended by force: no summary, and the note.
  { name: "review-aborted", ready: "role=note" },
  { name: "review-error", ready: "role=button[name='Retry']" },
  // History (§5.6, #529): the ride list in each of its states.
  { name: "history-empty", ready: "text=No rides yet." },
  { name: "history-error", ready: "role=button[name='Retry']" },
  { name: "history", ready: "role=table[name='Rides']" },
  {
    name: "history-long",
    ready: "role=table[name='Rides']",
    scrollTo: ".history-row:last-child",
  },
  // Back from a ride's Review: the list marks the ride last opened.
  {
    name: "history-selected",
    ready: "[aria-current='true']",
    clicks: ["All rides"],
  },
  { name: "settings", ready: "role=heading[name='Appearance']" },
  { name: "settings-athlete", ready: "role=heading[name='Athlete model']" },
  // A load finished while the rider was elsewhere: the ride-load notice offers it (#320).
  {
    name: "settings-ride-loaded",
    ready: "role=button[name='Go to ride']",
  },
  // The cold start (BRAND.md §Start screen, #552): a slow boot (status line at 1.2 s) and a failed one (the P6
  // alert), both at rest on the wordmark with the build's version string at the bottom right.
  {
    name: "start-slow",
    ready: "role=status >> text=Starting crank",
    noControls: true,
  },
  { name: "start-failed", ready: "role=alert >> text=can’t start" },
] as const satisfies readonly Scenario[];

export type ScenarioName = (typeof SCENARIOS)[number]["name"];

/** The fixtures of Today (#530): nothing on them is cut, and every control is inside the window. */
export const TODAY_SCENARIOS: readonly ScenarioName[] = [
  "today",
  "today-empty",
  "today-reading",
  "today-error",
  "today-ride-loading",
  "today-ride-loaded",
  "today-ride-refused",
  "today-link-lost",
  "today-links-warming",
  "today-links-not-found",
];

/** The fixtures of Settings and Devices (#527): nothing on them is cut, and every control is inside the window. */
export const SETTINGS_SCENARIOS: readonly ScenarioName[] = [
  "settings",
  "settings-athlete",
  "settings-ride-loaded",
  "devices-empty",
  "devices-scanning",
  "devices-found",
  "devices-pairing",
  "devices-paired",
  "devices-forget-confirm",
  "devices-denied",
];

/** The fixtures of Workouts (#528): nothing on them is cut, and every control is inside the window. */
export const WORKOUTS_SCENARIOS: readonly ScenarioName[] = [
  "workouts-empty",
  "workouts-list-error",
  "workouts",
  "workouts-selected",
  "workouts-links",
  "workouts-blocked",
  "workouts-loading",
  "workouts-load-error",
  "workouts-long",
  "workouts-editor",
  "workouts-editor-invalid",
  "workouts-mine",
  "workouts-delete-confirm",
  "workouts-editor-new",
];

/** The fixtures of History and Review (#529): nothing on them is cut, and every control is inside the window. */
export const HISTORY_SCENARIOS: readonly ScenarioName[] = [
  "review",
  "review-short",
  "review-long",
  "review-aborted",
  "review-error",
  "history-empty",
  "history-error",
  "history",
  "history-long",
  "history-selected",
];

/** The fixtures of the ride screen: their region boxes are the same at each size (DESIGN §18.7 rule 10, #498). */
export const RIDE_SCENARIOS: readonly ScenarioName[] = [
  "ready",
  "ready-loading",
  "ready-loading-progress",
  "ready-connecting",
  "ready-trainer-missing",
  "ready-strap-missing",
  "ready-error",
  "ride-soft-start",
  "ride-work-step",
  "ride-hr-zone",
  "ride-hr-zone-max",
  "ride-short-block",
  "ride-paused",
  "ride-hr-lost",
  "ride-trainer-lost",
  "ride-erg-released",
  "ride-free",
  "ride-trainer-lost-long",
  "ride-engine-silent",
  "ride-paused-hr-lost",
];
