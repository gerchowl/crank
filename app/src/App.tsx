// Shell: four places plus the ride / onboarding / resume modes (DESIGN §3), the labelled icon rail (§16.3),
// a top-level error boundary, theme, the `?` keys dialog and the cold-start hand-off (BRAND.md §Start screen).
import { useCallback, useEffect, useState, type ComponentType } from "react";
import { waitForCore as defaultWaitForCore, type WaitForCore } from "./core";
import {
  isMode,
  navigate,
  PLACES,
  useRoute,
  type Place,
  type View,
} from "./router";
import { ErrorBoundary } from "./shell/ErrorBoundary";
import { KeysDialog } from "./shell/KeysDialog";
import { StartScreen, type Boot } from "./shell/StartScreen";
import { useApplyTheme } from "./theme";
import { Placeholder } from "./views/Placeholder";
import { Settings } from "./views/Settings";

export type ViewComponent = ComponentType<{ sub?: string }>;
export type ViewRegistry = Record<View, ViewComponent>;

const soon =
  (title: string): ViewComponent =>
  () => <Placeholder title={title} />;
const DEFAULT_VIEWS: ViewRegistry = {
  today: soon("Today"),
  workouts: soon("Workouts"),
  history: soon("History"),
  settings: Settings,
  ride: soon("Ride"),
  onboarding: soon("Welcome"),
  resume: soon("Resume your ride"),
};

const RAIL: Record<Place, [label: string, icon: string]> = {
  today: ["Today", "◉"],
  workouts: ["Workouts", "▤"],
  history: ["History", "◷"],
  settings: ["Settings", "⚙"],
};

function useBoot(wait: WaitForCore) {
  const [boot, setBoot] = useState<Boot>({ phase: "booting", attempt: 1 });
  const attempt = boot.phase === "booting" ? boot.attempt : 0;
  useEffect(() => {
    if (!attempt) return;
    let live = true;
    wait().then(
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
  }, [wait, attempt]);
  const retry = useCallback(
    () =>
      setBoot((b) => ({
        phase: "booting",
        attempt: (b.phase === "booting" ? b.attempt : 0) + 1,
      })),
    [],
  );
  return { boot, retry };
}

function useKeysDialog() {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target;
      const typing =
        t instanceof HTMLElement &&
        (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
      if (e.key === "?" && !typing && !e.ctrlKey && !e.metaKey && !e.altKey)
        setOpen(true);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);
  return { open, close: useCallback(() => setOpen(false), []) };
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
  // `data-destination`: where focus lands when the start screen hands off; `#main` is the skip-link target.
  if (isMode(route.view))
    return (
      <main id="main" className="mode" data-destination tabIndex={-1}>
        {body}
      </main>
    );
  return (
    <div className="shell">
      <nav className="rail" aria-label="Places">
        <span className="brand">crank</span>
        {PLACES.map((p) => (
          <button
            key={p}
            type="button"
            aria-current={route.view === p ? "page" : undefined}
            onClick={() => navigate(p)}
          >
            <span className="rail-icon" aria-hidden>
              {RAIL[p][1]}
            </span>
            <span className="rail-label">{RAIL[p][0]}</span>
          </button>
        ))}
      </nav>
      <main id="main" data-destination tabIndex={-1}>
        {body}
      </main>
    </div>
  );
}

export function App({
  waitForCore = defaultWaitForCore,
  views = DEFAULT_VIEWS,
}: {
  /** The core hand-off adapter (core.ts); a test double in tests. */
  waitForCore?: WaitForCore;
  views?: ViewRegistry;
}) {
  const mode = useApplyTheme();
  const { boot, retry } = useBoot(waitForCore);
  const keys = useKeysDialog();
  const [gone, setGone] = useState(false);
  const onGone = useCallback(() => setGone(true), []);
  const skip = (e: React.MouseEvent) => {
    e.preventDefault();
    document.getElementById("main")?.focus();
  };
  return (
    <>
      <a className="skip-link" href="#main" onClick={skip}>
        Skip to main
      </a>
      {boot.phase === "ready" && <Destination views={views} />}
      {!gone && (
        <StartScreen boot={boot} mode={mode} onRetry={retry} onGone={onGone} />
      )}
      {keys.open && <KeysDialog onClose={keys.close} />}
    </>
  );
}
