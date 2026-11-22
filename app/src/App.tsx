// Shell: four places plus the ride / onboarding / resume modes (DESIGN §3), the labelled icon rail (§16.3),
// a top-level error boundary, theme, the `?` keys dialog and the cold-start hand-off (BRAND.md §Start screen).
import { useCallback, useEffect, useState, type ComponentType } from "react";
import {
  markSeen,
  readBuild as defaultReadBuild,
  seenBuild,
  type ReadBuild,
} from "./build";
import { waitForCore as defaultWaitForCore, type WaitForCore } from "./core";
import { isMode, navigate, useRoute, type View } from "./router";
import { ErrorBoundary } from "./shell/ErrorBoundary";
import { Rail } from "./shell/Rail";
import { KeysDialog } from "./shell/KeysDialog";
import { closeKeys, openKeys, useKeysOpen } from "./shell/keys";
import { StartScreen, reducedMotion, type Boot } from "./shell/StartScreen";
import { useApplyTheme } from "./theme";
import { Ride } from "./ride/Ride";
import { UiProvider } from "./stream/uiHooks";
import { LinkIntents } from "./links/LinkIntents";
import { History } from "./views/History";
import { Placeholder } from "./views/Placeholder";
import { RideLoadNotice } from "./views/RideLoadNotice";
import { ChannelNotice } from "./shell/ChannelNotice";
import { Settings } from "./views/Settings";
import { Today } from "./views/Today";
import { Workouts } from "./views/Workouts";

export type ViewComponent = ComponentType<{ sub?: string }>;
export type ViewRegistry = Record<View, ViewComponent>;

const soon =
  (title: string): ViewComponent =>
  () => <Placeholder title={title} />;
const DEFAULT_VIEWS: ViewRegistry = {
  today: Today,
  workouts: Workouts,
  history: History,
  settings: Settings,
  ride: Ride,
  onboarding: soon("Welcome"),
  resume: soon("Resume your ride"),
};

/** A boot attempt: the core's hand-off and the build's version string; ready needs both. */
function useBoot(wait: WaitForCore, read: ReadBuild) {
  const [boot, setBoot] = useState<Boot>({ phase: "booting", attempt: 1 });
  const [build, setBuild] = useState<string | null>(null);
  const attempt = boot.phase === "booting" ? boot.attempt : 0;
  useEffect(() => {
    if (!attempt) return;
    let live = true;
    const label = read().then((b) => {
      if (live) setBuild(b);
    });
    Promise.all([wait(), label]).then(
      () => live && setBoot({ phase: "ready" }),
      (e: unknown) =>
        live &&
        setBoot({
          phase: "failed",
          detail: e instanceof Error ? e.message : String(e),
        }),
    );
    return () => {
      live = false;
    };
  }, [wait, read, attempt]);
  const retry = useCallback(
    () =>
      setBoot((b) => ({
        phase: "booting",
        attempt: (b.phase === "booting" ? b.attempt : 0) + 1,
      })),
    [],
  );
  return { boot, build, retry };
}

/**
 * The once-per-version spin-up (#552, BRAND.md §Start screen): the first launch of a build holds the hand-off until
 * F · Spin-up has played through; once it has, that build is recorded as seen and later launches hand off the moment
 * the core is ready. Reduced motion has no motion to wait for: never held, nothing recorded.
 */
function useSpinup(build: string | null) {
  const [reduced] = useState(reducedMotion);
  const [seen] = useState(seenBuild);
  const [played, setPlayed] = useState(false);
  const onMotionEnd = useCallback(() => setPlayed(true), []);
  useEffect(() => {
    if (played && build !== null && build !== seen) markSeen(build);
  }, [played, build, seen]);
  const hold = !reduced && !played && build !== seen;
  return { hold, onMotionEnd };
}

function useKeysDialog() {
  const open = useKeysOpen();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target;
      const typing =
        t instanceof HTMLElement &&
        (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
      if (e.key === "?" && !typing && !e.ctrlKey && !e.metaKey && !e.altKey)
        openKeys();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);
  return { open, close: closeKeys };
}

function Destination({ views }: { views: ViewRegistry }) {
  const route = useRoute();
  const View = views[route.view];
  const body = (
    // Keyed by view: leaving a failed view clears the boundary.
    <ErrorBoundary key={route.view} onBack={() => navigate("today")}>
      <View sub={route.sub} />
    </ErrorBoundary>
  );
  // `data-destination`: where focus lands when the start screen hands off; `#main` is the skip-link target. The live-data
  // notice (stale channel, ChannelNotice.tsx) and the ride-load notice (#320) lead the main region, above the view; in a
  // mode (the ride screen) the stale state is the screen's own banner.
  if (isMode(route.view))
    return (
      <main id="main" className="mode" data-destination tabIndex={-1}>
        <RideLoadNotice />
        {body}
      </main>
    );
  return (
    <div className="shell">
      <Rail view={route.view} />
      <main id="main" data-destination tabIndex={-1}>
        <ChannelNotice />
        <RideLoadNotice />
        {body}
      </main>
    </div>
  );
}

export function App({
  waitForCore = defaultWaitForCore,
  readBuild = defaultReadBuild,
  views = DEFAULT_VIEWS,
}: {
  /** The core hand-off adapter (core.ts); a test double in tests. */
  waitForCore?: WaitForCore;
  /** The build's version string (build.ts); a test double in tests and fixtures. */
  readBuild?: ReadBuild;
  views?: ViewRegistry;
}) {
  const mode = useApplyTheme();
  const { boot, build, retry } = useBoot(waitForCore, readBuild);
  const spinup = useSpinup(build);
  // Ready implies the build string is known (useBoot), so `hold` is decided by then.
  const handoff = boot.phase === "ready" && !spinup.hold;
  const keys = useKeysDialog();
  const [gone, setGone] = useState(false);
  const onGone = useCallback(() => setGone(true), []);
  const skip = (e: React.MouseEvent) => {
    e.preventDefault();
    document.getElementById("main")?.focus();
  };
  return (
    <>
      {handoff && (
        // Nothing to skip to before the destination exists: the start screen has no repeated blocks (WCAG 2.4.1).
        <a className="skip-link" href="#main" onClick={skip}>
          Skip to main
        </a>
      )}
      {/* The window's snapshot channel opens with the destination, once the core is up and the start screen hands off,
          and stays open for the window's life (ADR-0001 §4.3): every view reads the live state from it. The link keeper
          hears the window's intent from then on too (ADR-0002 §5.1.2): the view, focus, visibility and rider input. */}
      {handoff && (
        <UiProvider>
          <LinkIntents>
            <Destination views={views} />
          </LinkIntents>
        </UiProvider>
      )}
      {!gone && (
        <StartScreen
          boot={boot}
          mode={mode}
          build={build}
          leaving={handoff}
          onRetry={retry}
          onGone={onGone}
          onMotionEnd={spinup.onMotionEnd}
        />
      )}
      {keys.open && <KeysDialog onClose={keys.close} />}
    </>
  );
}

/** The outermost boundary (DESIGN §10 #4): a throw in the rail, start screen or dialog is a panel, not a white window. */
export function AppRoot(props: Parameters<typeof App>[0]) {
  return (
    <ErrorBoundary onBack={() => navigate("today")}>
      <App {...props} />
    </ErrorBoundary>
  );
}
