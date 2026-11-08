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
}

export const SCENARIOS = [
  { name: "today", ready: "role=heading[name='Today']" },
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
  // The post-ride summary: a finished ride routes to History with its id until the summary route exists (#106).
  { name: "review", ready: "role=heading[name='Sweet spot 3x12']" },
  { name: "history", ready: "role=list[name='Rides']" },
  { name: "settings", ready: "role=heading[name='Appearance']" },
] as const satisfies readonly Scenario[];

export type ScenarioName = (typeof SCENARIOS)[number]["name"];

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
