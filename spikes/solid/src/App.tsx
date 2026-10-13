import { For, lazy, Match, Suspense, Switch } from "solid-js";
import { RideScreen } from "./ride/RideScreen";
import { router, type View } from "./state/router";
import { ThemeMenu } from "./ui/ThemeMenu";

const Editor = lazy(() => import("./editor/Editor").then((m) => ({ default: m.Editor })));
const Checkin = lazy(() => import("./checkin/Checkin").then((m) => ({ default: m.Checkin })));
const Review = lazy(() => import("./review/Review").then((m) => ({ default: m.Review })));

const NAV: { view: View; label: string; icon: string }[] = [
  { view: "checkin", label: "Check-in", icon: "☀" },
  { view: "ride", label: "Ride", icon: "▶" },
  { view: "editor", label: "Editor", icon: "✎" },
  { view: "review", label: "Review", icon: "▤" },
];

/** Ride is a full-screen modal (no rail); the other views sit next to a left rail. */
export function App() {
  return (
    <Switch>
      <Match when={router.view() === "ride"}>
        <RideScreen />
      </Match>
      <Match when={router.view() !== "ride"}>
        <div class="shell">
          <nav class="rail" aria-label="Main">
            <div class="brand">neo</div>
            <For each={NAV}>
              {(n) => (
                <button class="rail-item" classList={{ on: router.view() === n.view }} onClick={() => router.go(n.view)}>
                  <span class="rail-icon">{n.icon}</span>
                  {n.label}
                </button>
              )}
            </For>
            <div class="rail-spacer" />
            <ThemeMenu />
          </nav>
          <div class="view">
            <Suspense fallback={<div class="empty">Loading…</div>}>
              <Switch>
                <Match when={router.view() === "checkin"}>
                  <Checkin />
                </Match>
                <Match when={router.view() === "editor"}>
                  <Editor />
                </Match>
                <Match when={router.view() === "review"}>
                  <Review />
                </Match>
              </Switch>
            </Suspense>
          </div>
        </div>
      </Match>
    </Switch>
  );
}
