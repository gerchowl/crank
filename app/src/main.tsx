import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { AppRoot } from "./App";
import "./theme.css";
import "./shell.css";
const root = document.getElementById("root");
if (!root) throw new Error("Missing app root");
createRoot(root).render(
  <StrictMode>
    <AppRoot />
  </StrictMode>,
);
