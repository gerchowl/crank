import { readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { FALLBACK_TOKENS } from "./typeTiers";

// stylelint.config.mjs lists the custom properties that useTypeTiers sets at runtime (no stylesheet declares
// them), so `value-no-unknown-custom-properties` accepts them. That list must be exactly the tokens it sets.
test("the stylelint runtime-token list is the type-tier tokens", () => {
  const config = readFileSync(
    new URL("../stylelint.config.mjs", import.meta.url),
    "utf8",
  );
  const list = /RUNTIME_TOKENS = \[([^\]]*)\]/.exec(config)?.[1] ?? "";
  const names = [...list.matchAll(/"(--[\w-]+)"/g)].map((m) => m[1]);
  expect(names.sort()).toEqual(Object.keys(FALLBACK_TOKENS).sort());
});
