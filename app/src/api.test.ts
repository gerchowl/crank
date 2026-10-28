import { expect, test } from "vitest";
import { api, toApiError } from "./api";

test("an ApiError passes through unchanged", () => {
  const e = { code: "not_found", message: "no ride" };
  expect(toApiError(e)).toBe(e);
});

test("Tauri's plain-string rejections become Internal with the text kept", () => {
  expect(toApiError("invalid args `id` for command `ride_summary`")).toEqual({
    code: "internal",
    message: "invalid args `id` for command `ride_summary`",
  });
});

test("other shapes become Internal", () => {
  expect(toApiError(new Error("boom"))).toEqual({
    code: "internal",
    message: "boom",
  });
  expect(toApiError({ code: 5 })).toEqual({
    code: "internal",
    message: '{"code":5}',
  });
  expect(toApiError(null).code).toBe("internal");
});

test("api maps an error result and keeps an ok one", async () => {
  const bad = await api(
    Promise.resolve({ status: "error" as const, error: "unknown command" }),
  );
  expect(bad).toEqual({
    status: "error",
    error: { code: "internal", message: "unknown command" },
  });
  const ok = await api(Promise.resolve({ status: "ok" as const, data: 3 }));
  expect(ok).toEqual({ status: "ok", data: 3 });
});
