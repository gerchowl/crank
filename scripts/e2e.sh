#!/usr/bin/env bash
# `just e2e` (ADR-0001 §8.3, #35): build crank-desktop-e2e with Nix and run the WebdriverIO specs.
# A WebView needs the logged-in GUI session, which flock panes do not have, so the run is handed to
# Alacritty over IPC (the BLE-run pattern) and its log and exit code are read back from files.
# CRANK_E2E_DIRECT=1 runs in the current terminal instead (use it from a GUI terminal).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=${CRANK_E2E_OUT:-$root/app/e2e-out}
mkdir -p "$out"
rm -f "$out/exit" "$out/log"

nix build "$root#crank-desktop-e2e" --out-link "$out/app"
app=$(readlink -f "$out/app")/bin/crank-desktop

if [ "${CRANK_E2E_DIRECT:-}" = 1 ]; then
  CRANK_E2E_APP=$app CRANK_E2E_OUT=$out pnpm -C "$root/app" exec wdio run e2e/wdio.conf.mjs
  exit
fi

wrapper=$out/run.sh
cat > "$wrapper" <<EOF
#!/usr/bin/env bash
export PATH='$PATH'
export CRANK_E2E_APP='$app' CRANK_E2E_OUT='$out'
cd '$root'
pnpm -C app exec wdio run e2e/wdio.conf.mjs > '$out/log' 2>&1
echo \$? > '$out/exit'
EOF
chmod +x "$wrapper"
# nix develop repoints TMPDIR; Alacritty creates its socket in the per-user temp dir.
socket=$(ls -t "$(getconf DARWIN_USER_TEMP_DIR)"Alacritty-*.sock 2>/dev/null | head -1 || true)
[ -n "$socket" ] || { echo "e2e: no Alacritty IPC socket: open Alacritty in the GUI session first" >&2; exit 1; }
alacritty msg --socket "$socket" create-window -e "$wrapper"
for _ in $(seq 1 600); do
  [ -f "$out/exit" ] && break
  sleep 1
done
[ -f "$out/exit" ] || { echo "e2e: no result after 600 s" >&2; cat "$out/log" 2>/dev/null; exit 1; }
cat "$out/log"
code=$(cat "$out/exit")
[ "$code" = 0 ] && (cd "$out" && shasum -a 256 shell-1.png shell-2.png)
exit "$code"
