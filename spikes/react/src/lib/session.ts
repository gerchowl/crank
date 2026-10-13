// Cross-screen hand-off (check-in → editor, editor → ride). Not telemetry; plain external store.
import type { Workout } from "@neo/shared";
import { createStore } from "./store";

export const editorSeed = createStore<Workout | null>(null);
