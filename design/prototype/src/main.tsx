import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./theme.css";
import "./app.css";
import { App } from "./App";
import { applyScene, scenes } from "./scenes";
import { navigate, sceneId } from "./session";
import { safety } from "./safety";
import { startBoot } from "./boot";

const params = new URLSearchParams(location.search);
const t = params.get("theme");
const forcedTheme = t === "dark" || t === "light" ? t : null;

if (params.has("list-scenes")) document.body.dataset.scenes = JSON.stringify(Object.keys(scenes));

// ?stop=1: start with a medical stop on record (DESIGN §5.8; used by scripts/safety.test.mjs)
if (params.get("stop") === "1") safety.set((s) => ({ ...s, stop: true }));

let sceneError: string | null = null;
if (sceneId) {
  try {
    applyScene(sceneId);
  } catch (e) {
    sceneError = e instanceof Error ? e.message : String(e);
  }
} else if (params.get("first") === "1") navigate("onboarding");

const el = document.getElementById("root");
if (!el) throw new Error("#root missing");
if (sceneError) {
  el.textContent = `scene error: ${sceneError}`;
  el.dataset.sceneError = sceneError;
} else {
  createRoot(el).render(
    <StrictMode>
      <App forcedTheme={forcedTheme} />
    </StrictMode>,
  );
  startBoot();
}
