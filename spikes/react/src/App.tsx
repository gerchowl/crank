import { lazy, Suspense } from "react";
import { useRoute, go, type Route } from "./lib/router";
import { RideScreen } from "./ride/RideScreen";
import { ThemeMenu } from "./theme/ThemeMenu";

const Editor = lazy(() => import("./editor/Editor").then((m) => ({ default: m.Editor })));
const Checkin = lazy(() => import("./checkin/Checkin").then((m) => ({ default: m.Checkin })));
const Review = lazy(() => import("./review/Review").then((m) => ({ default: m.Review })));

const NAV: [Route, string, string][] = [
  ["checkin", "Check-in", "☀"],
  ["ride", "Ride", "▶"],
  ["editor", "Editor", "✎"],
  ["review", "Review", "◷"],
];

export function App() {
  const route = useRoute();
  // Ride is a full-screen modal (no rail).
  if (route === "ride") return <RideScreen />;
  return (
    <div className="shell">
      <nav className="rail" aria-label="main">
        <div className="brand">neo</div>
        {NAV.map(([r, label, icon]) => (
          <button type="button" key={r} className={r === route ? "on" : ""} onClick={() => { go(r); }}>
            <span aria-hidden>{icon}</span>
            {label}
          </button>
        ))}
        <span className="spacer" />
        <ThemeMenu />
      </nav>
      <main className="screen">
        <Suspense fallback={<p className="muted pad">Loading…</p>}>
          {route === "editor" && <Editor />}
          {route === "checkin" && <Checkin />}
          {route === "review" && <Review />}
        </Suspense>
      </main>
    </div>
  );
}
