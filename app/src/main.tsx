import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { AppRoot } from "./App";
import "./theme.css";
import "./shell.css";
import { windowUi } from "./stream/ui";

// `just e2e` reads the page's own live state (app/e2e/specs/ui.e2e.mjs): a read-only view of the snapshot channel's
// client, the object the views render. It changes nothing and exists in every build: the e2e app is the production
// frontend (flake.nix `appDist`). ADR-0001 §4.3.2 "The e2e probe" records it.
Object.defineProperty(window, "__crankUi", { value: () => windowUi.snapshot });
const root = document.getElementById("root");
if (!root) throw new Error("Missing app root");
createRoot(root).render(
  <StrictMode>
    <AppRoot />
  </StrictMode>,
);
