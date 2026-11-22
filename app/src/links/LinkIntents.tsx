// Binds the window's intent reporter (intent.ts, ADR-0002 §5.1.2) to the route and the snapshot channel. Mounted once
// around the destination, for the window's life; it renders its children and gives the link chips their Retry
// (chips.tsx `RetryContext`), which goes through the same reporter.
import { useEffect, useState, type ReactNode } from "react";
import { useRoute } from "../router";
import { useUi } from "../stream/uiHooks";
import { RetryContext } from "./chips";
import { IntentReporter, viewIntent } from "./intent";

export function LinkIntents({ children }: { children?: ReactNode }) {
  const route = useRoute();
  const ui = useUi();
  const [reporter] = useState(() => new IntentReporter());
  const [retry] = useState(() => () => reporter.retry());
  useEffect(() => reporter.start(), [reporter]);
  const intent = viewIntent(route, ui);
  // The reporter sends only a change, so a render that changes nothing (the ride's 1 s heartbeat) sends nothing.
  useEffect(() => reporter.setView(intent), [reporter, intent]);
  return <RetryContext value={retry}>{children}</RetryContext>;
}
