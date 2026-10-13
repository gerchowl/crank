// Single app-wide backend instance (stands in for the Tauri channel to the Rust core).
// Only the `Backend` contract is used by the UI; see adapter.ts for the gaps we work around.
import { createMockBackend, type Backend } from "@neo/shared";

const params = new URLSearchParams(location.search);
const speedParam = Number(params.get("speed") ?? "10");
const speed: 1 | 10 | 60 = speedParam === 1 || speedParam === 60 ? speedParam : 10;

export const backend: Backend = createMockBackend({ speed, autoStart: params.get("autostart") !== "0" });
