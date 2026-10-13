import { useSyncExternalStore } from "react";

export type Route = "checkin" | "ride" | "editor" | "review";
const ROUTES: readonly Route[] = ["checkin", "ride", "editor", "review"];

function read(): Route {
  const h = location.hash.replace(/^#\/?/, "").split("?")[0] ?? "";
  return ROUTES.find((r) => r === h) ?? "ride";
}
function subscribe(cb: () => void) {
  addEventListener("hashchange", cb);
  return () => {
    removeEventListener("hashchange", cb);
  };
}
export function useRoute(): Route {
  return useSyncExternalStore(subscribe, read);
}
export function go(r: Route) {
  location.hash = `/${r}`;
}
