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
  /** Mock answers that stay pending on purpose (a scan in flight): the settle wait ignores this many. */
  held?: number;
  /** A selector the spec scrolls into view once the state is ready (a long list, scrolled to its end). */
  scrollTo?: string;
}

export const SCENARIOS = [
  // Today (§5.1 "In direction A", #530). `today` stays first: the gate's own tests inject into it.
  { name: "today", ready: "role=list[name='Last 7 days']" },
  // First run: no rides, nothing paired; the devices say a ride needs a trainer.
  { name: "today-empty", ready: "text=No rides yet" },
  // The reads are still on the wire: StrictMode runs each mount effect twice, so each read is asked twice.
  { name: "today-reading", ready: "text=Loading…", held: 4 },
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
  { name: "ready-loading", ready: "[data-ready='loading']", held: 1 },
  { name: "ready-connecting", ready: "[data-ready='connecting']" },
  { name: "ready-trainer-missing", ready: "[data-ready='trainer-missing']" },
  { name: "ready-strap-missing", ready: "[data-ready='strap-missing']" },
  { name: "ready-error", ready: "[data-ready='error']", clicks: ["Start"] },
  { name: "ride-soft-start", ready: "[data-testid='soft-start']" },
  {
    name: "ride-work-step",
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
];

/** The fixtures of Settings and Devices (#527): nothing on them is cut, and every control is inside the window. */
export const SETTINGS_SCENARIOS: readonly ScenarioName[] = [
  "settings",
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
  "workouts-blocked",
  "workouts-loading",
  "workouts-load-error",
  "workouts-long",
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
  "ready-connecting",
  "ready-trainer-missing",
  "ready-strap-missing",
  "ready-error",
  "ride-soft-start",
  "ride-work-step",
  "ride-paused",
  "ride-hr-lost",
  "ride-trainer-lost",
  "ride-erg-released",
  "ride-free",
  "ride-trainer-lost-long",
  "ride-engine-silent",
  "ride-paused-hr-lost",
];
