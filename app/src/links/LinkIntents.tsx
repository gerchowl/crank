// Binds the window's intent reporter (intent.ts, ADR-0002 §5.1.2) to the route and the snapshot channel: mounted once
// beside the destination, for the window's life, it renders nothing.
import { useEffect, useState } from "react";
import { useRoute } from "../router";
import { useUi } from "../stream/uiHooks";
import { IntentReporter, viewIntent } from "./intent";

export function LinkIntents() {
  const route = useRoute();
  const ui = useUi();
  const [reporter] = useState(() => new IntentReporter());
  useEffect(() => reporter.start(), [reporter]);
  const intent = viewIntent(route, ui);
  // The reporter sends only a change, so a render that changes nothing (the ride's 1 s heartbeat) sends nothing.
  useEffect(() => reporter.setView(intent), [reporter, intent]);
  return null;
}
