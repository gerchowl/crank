// Shell: four places + the ride mode (DESIGN §3), a top-level error boundary (#4).
import { Component, useCallback, useEffect, useState, type ReactNode } from "react";
import { boot } from "./boot";
import { StartScreen } from "./StartScreen";
import { theme } from "./persist";
import { navigate, useRoute, type View } from "./session";
import { useStore } from "./store";
import { Onboarding } from "./views/Onboarding";
import { Review, History } from "./views/Review";
import { Ride } from "./views/Ride";
import { Settings } from "./views/Settings";
import { Ready, Today } from "./views/Today";
import { Resume } from "./views/Resume";
import { Gate } from "./views/Gate";
import { Workouts } from "./views/Workouts";
import { WhySheet } from "./views/Why";

const PLACES: [View, string, string][] = [
  ["today", "Today", "◉"],
  ["workouts", "Workouts", "▤"],
  ["history", "History", "◷"],
  ["settings", "Settings", "⚙"],
];

class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  override state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  override render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="page" role="alert">
        <h1>Something went wrong</h1>
        <p className="muted">{this.state.error.message}</p>
        <button className="primary" onClick={() => (this.setState({ error: null }), navigate("today"))}>
          Back to Today
        </button>
      </div>
    );
  }
}

function Screen({ view }: { view: View }) {
  switch (view) {
    case "onboarding":
      return <Onboarding />;
    case "resume":
      return <Resume />;
    case "gate":
      return <Gate />;
    case "today":
      return <Today />;
    case "ready":
      return <Ready />;
    case "ride":
      return <Ride />;
    case "review":
      return <Review />;
    case "history":
      return <History />;
    case "workouts":
      return <Workouts />;
    case "settings":
      return <Settings />;
  }
}

export function App({ forcedTheme }: { forcedTheme: "dark" | "light" | null }) {
  const b = useStore(boot);
  const saved = useStore(theme);
  const mode = forcedTheme ?? saved;
  useEffect(() => {
    document.documentElement.dataset.theme = mode;
  }, [mode]);
  // The start screen exists only if the app launched while the core was still booting (no minimum display
  // time: a core that is ready before the first frame never shows it). It stays up through the hand-off fade.
  const [coldStart] = useState(b.phase !== "ready");
  const [gone, setGone] = useState(false);
  const onGone = useCallback(() => {
    delete document.documentElement.dataset.start; // the pre-paint window background (index.html) ends with the start screen
    setGone(true);
  }, []);
  return (
    <>
      {b.phase === "ready" && <Destination />}
      {coldStart && !gone && <StartScreen boot={b} mode={mode} onGone={onGone} />}
    </>
  );
}

function Destination() {
  const r = useRoute();
  const place = r.view === "ready" ? "today" : r.view === "review" ? "history" : r.view;
  if (r.view === "ride") return <Ride />;
  // Onboarding and the Resume sheet are modes: no rail, so there is nowhere half-set-up to wander (§3).
  // `data-destination`: where focus lands when the start screen hands off.
  if (r.view === "onboarding" || r.view === "resume")
    return (
      <main className="mode" data-destination tabIndex={-1}>
        <ErrorBoundary key={r.view}>
          <Screen view={r.view} />
        </ErrorBoundary>
      </main>
    );
  return (
    <div className="shell">
      <nav className="rail" aria-label="Places">
        <span className="brand">crank</span>
        {PLACES.map(([v, label, icon]) => (
          <button key={v} className={place === v ? "on" : ""} aria-current={place === v ? "page" : undefined} onClick={() => navigate(v)}>
            <span aria-hidden>{icon}</span>
            {label}
          </button>
        ))}
      </nav>
      <main data-destination tabIndex={-1}>
        <ErrorBoundary key={r.view}>
          <Screen view={r.view} />
        </ErrorBoundary>
      </main>
      <WhySheet />
    </div>
  );
}
