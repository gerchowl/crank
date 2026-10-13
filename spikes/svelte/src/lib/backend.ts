// The one place the app creates its backend. Everything else imports `backend` and sees only
// the `Backend` contract. URL params exist for demos and the screenshot script:
//   ?speed=1|10|60 (default 10)  ?autoplay=0 (don't start the demo ride)  ?seed=n
import { createMockBackend, type Backend } from "@neo/shared";

const params = new URLSearchParams(location.search);

function speedParam(): 1 | 10 | 60 {
  const s = params.get("speed");
  return s === "1" ? 1 : s === "60" ? 60 : 10;
}

const mock = createMockBackend({
  seed: Number(params.get("seed") ?? 42),
  speed: speedParam(),
  autoStart: params.get("autoplay") !== "0",
});

export const backend: Backend = mock;

if (import.meta.env.DEV) {
  // Screenshot script hook: lets Playwright skip blocks without clicking through the UI.
  (window as unknown as { __neo?: Backend }).__neo = backend;
}
