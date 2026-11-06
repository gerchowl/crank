// Whether the `?` keys dialog is open (#88): one flag for the window, so the `?` key (App) and a screen's own Keys
// button (the ride's key row, DESIGN §18.3) open the same dialog.
import { useSyncExternalStore } from "react";

let open = false;
const listeners = new Set<() => void>();
const set = (v: boolean) => {
  open = v;
  for (const l of [...listeners]) l();
};

export const openKeys = () => set(true);
export const closeKeys = () => set(false);

export const useKeysOpen = (): boolean =>
  useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    () => open,
  );
