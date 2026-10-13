import { createRoot, createSignal } from "solid-js";

export type View = "ride" | "checkin" | "editor" | "review";
const VIEWS: readonly View[] = ["ride", "checkin", "editor", "review"];

function parse(): View {
  const v = location.hash.replace(/^#\/?/, "").split("?")[0] ?? "";
  return (VIEWS as readonly string[]).includes(v) ? (v as View) : "ride";
}

/** Tiny hash router: one signal, so views mount/unmount via <Switch>. */
export const router = createRoot(() => {
  const [view, setView] = createSignal<View>(parse());
  window.addEventListener("hashchange", () => setView(parse()));
  const go = (v: View) => {
    if (parse() !== v) location.hash = `/${v}`;
    setView(v);
  };
  return { view, go };
});

/** Query params after the hash (e.g. #/ride?speed=60) — used for dev/screenshots. */
export function hashParam(name: string): string | null {
  const q = location.hash.split("?")[1];
  return q ? new URLSearchParams(q).get(name) : null;
}
