// Normalises a generated command result (src/bindings.ts, never hand-edited) so the UI sees one error shape.
// Rust returns `ApiError` for every failure the core knows (ADR-0001 §4.4). Tauri itself rejects with a
// plain string for what happens before the core is reached (malformed or missing arguments, an unknown
// command, a denied ACL), and the generated `typedError` passes that string through as `error`.
import type { ApiError } from "./bindings";

export type Result<T> =
  { status: "ok"; data: T } | { status: "error"; error: unknown };

/** Any rejection value as an `ApiError`; non-`ApiError` shapes become `Internal` with the text kept. */
export function toApiError(e: unknown): ApiError {
  if (
    typeof e === "object" &&
    e !== null &&
    "code" in e &&
    typeof e.code === "string" &&
    "message" in e &&
    typeof e.message === "string"
  ) {
    return e as ApiError;
  }
  const message =
    typeof e === "string"
      ? e
      : e instanceof Error
        ? e.message
        : JSON.stringify(e);
  return { code: "internal", message };
}

/** Await a generated command call; an error result carries an `ApiError` whatever its origin. */
export async function api<T>(
  call: Promise<Result<T>>,
): Promise<{ status: "ok"; data: T } | { status: "error"; error: ApiError }> {
  const result = await call;
  return result.status === "ok"
    ? result
    : { status: "error", error: toApiError(result.error) };
}
