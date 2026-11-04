// The design fixtures (#457): every screen and state the MVP uses, by the name `fixture.html?scenario=<name>` takes.
// Pure data, shared by the fixture entry (fixture.tsx, which sets each one up on the repo's mockIPC backends) and the
// Playwright spec (design.pw.ts, which opens, settles and audits each one). Each fixture is a static state, not a
// simulation: nothing ticks, so two runs render the same page.

export interface Scenario {
  /** The `?scenario=` value. */
  name: string;
  /** A Playwright selector that is visible once the state is on screen. */
  ready: string;
  /** Buttons the spec clicks, by accessible name, before it waits for `ready` (a pick, a scan). */
  clicks?: string[];
  /** Mock answers that stay pending on purpose (a scan in flight): the settle wait ignores this many. */
  held?: number;
}

export const SCENARIOS = [
  { name: "today", ready: "role=heading[name='Today']" },
  { name: "workouts", ready: "role=button[name=/Sweet spot 3x12/]" },
  {
    name: "workouts-selected",
    ready: "role=region[name='Compiled summary']",
    clicks: ["Sweet spot 3x12"],
  },
  { name: "devices-empty", ready: "role=button[name='Scan']" },
  {
    name: "devices-scanning",
    ready: "role=button[name='Scanning…']",
    clicks: ["Scan"],
    held: 1,
  },
  {
    name: "devices-paired",
    ready: "role=list[name='Nearby devices']",
    clicks: ["Scan"],
  },
  {
    name: "devices-denied",
    ready: "text=Bluetooth access is denied",
    clicks: ["Scan"],
  },
  // The loaded, unstarted ride: today's stand-in for the Ready sheet (§5.4, #438 step 4 with #435).
  { name: "ready", ready: "button:enabled:text-is('Start')" },
  { name: "ride-soft-start", ready: "[data-testid='soft-start']" },
  { name: "ride-work-step", ready: "button:enabled:text-is('Pause')" },
  { name: "ride-paused", ready: "button:enabled:text-is('Resume')" },
  { name: "ride-hr-lost", ready: "[data-testid='loss-hr']" },
  { name: "ride-trainer-lost", ready: "[data-testid='loss-trainer']" },
  // The post-ride summary: a finished ride routes to History with its id until the summary route exists (#106).
  { name: "review", ready: "role=heading[name='Sweet spot 3x12']" },
  { name: "history", ready: "role=list[name='Rides']" },
  { name: "settings", ready: "role=heading[name='Appearance']" },
] as const satisfies readonly Scenario[];

export type ScenarioName = (typeof SCENARIOS)[number]["name"];
