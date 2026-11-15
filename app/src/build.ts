// The build's version string and the once-per-version spin-up (#552, BRAND.md §Start screen). The string,
// `<version> · <rev>` (e.g. `0.2.0-rc.1 · 7516045`, `-dirty` for a dirty tree, `· dev` off the flake), is fixed in
// the desktop binary at build time (version.rs `build_label`) and read once per boot attempt; the page never
// formats or guesses it. "Seen" records the string whose spin-up has played through, in the app's local settings
// (localStorage, beside the theme, until the persistence primitive #90; never the store, so it does not sync):
// the first launch of a new install or update holds the start screen until the motion ends, later ones do not.
import { commands } from "./bindings";

/** Resolves with this build's version string; rejects when the desktop cannot be asked. */
export type ReadBuild = () => Promise<string>;

/** Default adapter: the desktop's `app_build` command. */
export const readBuild: ReadBuild = () => commands.appBuild();

const KEY = "crank.spinup.v1";

/** Accepts exactly `{"v":1,"build":"<non-empty string>"}`; anything else is no record. */
export function parseSeen(raw: string): string | null {
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof v !== "object" || v === null || Array.isArray(v)) return null;
  const keys = Object.keys(v).sort().join(",");
  if (keys !== "build,v") return null;
  const r = v as { v: unknown; build: unknown };
  return r.v === 1 && typeof r.build === "string" && r.build !== ""
    ? r.build
    : null;
}

/**
 * The build whose spin-up last played through, or null. Unreadable storage reads as none: the worst case is the
 * motion playing in full once more, which is what a first launch does anyway.
 */
export function seenBuild(): string | null {
  try {
    const raw = localStorage.getItem(KEY);
    return raw === null ? null : parseSeen(raw);
  } catch {
    return null;
  }
}

/**
 * Record that `build`'s spin-up played through. A failed write leaves the old record: the next launch plays the
 * motion in full again and retries the write; nothing else depends on it.
 */
export function markSeen(build: string): void {
  try {
    localStorage.setItem(KEY, JSON.stringify({ v: 1, build }));
  } catch {
    // See above: the cost of a lost record is one more full spin-up.
  }
}
