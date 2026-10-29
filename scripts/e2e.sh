#!/usr/bin/env bash
# `just e2e` (ADR-0001 §8.3, #35): build crank-desktop-e2e with Nix and run the WebdriverIO specs twice
# (two app launches) and compare the screenshots across them (§8.3.1).
# A WebView needs the logged-in GUI session, which flock panes do not have, so the run is handed to
# Alacritty over IPC (the BLE-run pattern) and its log and exit code are read back from files.
# CRANK_E2E_DIRECT=1 runs in the current terminal instead (use it from a GUI terminal).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=e2e-lib.sh
. "$root/scripts/e2e-lib.sh"

# Inside the wrapper (`--launches <dir>`): two launches, then compare. Used by both modes.
if [ "${1:-}" = --launches ]; then
  dir=$2
  for n in 1 2; do
    mkdir -p "$dir/launch-$n"
    # The simulator and a scratch data dir per launch: no hardware, and no ride.lock shared between launches.
    CRANK_SIM=1 CRANK_DATA_DIR=$dir/launch-$n/data CRANK_E2E_OUT=$dir/launch-$n pnpm -C "$root/app" exec wdio run e2e/wdio.conf.mjs
  done
  # Cross-launch determinism: same page, separate app launches, byte-identical screenshots.
  for shot in shell-1.png shell-2.png; do
    cmp "$dir/launch-1/$shot" "$dir/launch-2/$shot"
  done
  (cd "$dir/launch-1" && shasum -a 256 shell-1.png shell-2.png)
  exit 0
fi

if [ "$(uname -s)" != Darwin ]; then
  echo "e2e: skipped: the embedded WebDriver harness is validated on macOS only (ADR-0001 §8.3)" >&2
  exit 0
fi

out=${CRANK_E2E_OUT:-$root/app/e2e-out}
mkdir -p "$out"
# A per-run directory: a run that outlives the wait below can never leak its result into the next one.
run=$(mktemp -d "$out/run.XXXXXX")
# Keep the 3 newest earlier run dirs for post-mortems; older ones are removed.
prune_runs "$out" "$run" 3

nix build "$root#crank-desktop-e2e" --out-link "$out/app"
app=$(readlink -f "$out/app")/bin/crank-desktop
export CRANK_E2E_APP=$app

if [ "${CRANK_E2E_DIRECT:-}" = 1 ]; then
  "$0" --launches "$run"
  rm -rf "$run"
  exit 0
fi

wrapper=$run/run.sh
{
  echo '#!/usr/bin/env bash'
  printf 'export PATH=%q\n' "$PATH"
  printf 'export CRANK_E2E_APP=%q\n' "$app"
  printf 'cd %q\n' "$root"
  printf '%q --launches %q > %q 2>&1\n' "$root/scripts/e2e.sh" "$run" "$run/log"
  # Atomic: the poller below never sees a partly written exit file.
  printf 'echo $? > %q && mv %q %q\n' "$run/exit.tmp" "$run/exit.tmp" "$run/exit"
} > "$wrapper"
chmod +x "$wrapper"
# nix develop repoints TMPDIR; Alacritty creates its socket in the per-user temp dir.
socket=$(ls -t "$(getconf DARWIN_USER_TEMP_DIR)"Alacritty-*.sock 2>/dev/null | head -1 || true)
[ -n "$socket" ] || { echo "e2e: no Alacritty IPC socket: open Alacritty in the GUI session first" >&2; exit 1; }
alacritty msg --socket "$socket" create-window -e "$wrapper"
for _ in $(seq 1 1200); do
  [ -f "$run/exit" ] && break
  sleep 1
done
[ -f "$run/exit" ] || { echo "e2e: no result after 1200 s ($run)" >&2; cat "$run/log" 2>/dev/null; exit 1; }
cat "$run/log"
code=$(cat "$run/exit")
# A passing run leaves nothing behind; a failing one keeps its dir (log, screenshots).
[ "$code" = 0 ] && rm -rf "$run"
exit "$code"
