// React bindings of the window's snapshot channel (ADR-0001 §4.3). `UiProvider` is app-scoped: it opens the channel
// when the app mounts and holds it for the window's life, with or without a ride. The hooks read what the last frame
// said; nothing here derives state.
import {
  createContext,
  useContext,
  useEffect,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { windowUi, type UiClient, type UiSnapshot } from "./ui";

const UiContext = createContext<UiClient>(windowUi);

/** Open the window's snapshot channel while mounted (once, at the app's root). */
export function UiProvider({
  client = windowUi,
  children,
}: {
  client?: UiClient;
  children: ReactNode;
}) {
  useEffect(() => client.retain(), [client]);
  return <UiContext value={client}>{children}</UiContext>;
}

/** The window's live state, as the last frame of its snapshot channel said it. */
export function useUi(): UiSnapshot {
  const client = useContext(UiContext);
  return useSyncExternalStore(
    client.listen,
    () => client.snapshot,
    () => client.snapshot,
  );
}
