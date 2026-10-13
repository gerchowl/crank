import { expect, test } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { App } from "./App";
test("renders the placeholder without a Tauri runtime", () => {
  expect(renderToStaticMarkup(<App />)).toContain(
    "Your indoor cycling workspace",
  );
});
