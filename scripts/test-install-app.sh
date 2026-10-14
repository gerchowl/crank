#!/usr/bin/env bash
# Exercise scripts/install-app.sh against a temp target with a stub codesign (#165): fresh install,
# swap over an existing app, and a requirement mismatch that must leave the old app untouched.
# Limit (#165): this checks the swap's outcome only. Replacing renamex_np with `rm -rf; mv` would
# still pass, because a single-threaded script cannot observe the absent-path window between the
# two steps; atomicity is by construction (the exchange is one syscall), not asserted here.
set -euo pipefail
# install-app.sh swaps with renamex_np, which exists on Darwin only; the recipe is macOS-only by design
# (ADR-0001 §9.3), so there is nothing to exercise elsewhere.
if [ "$(uname -s)" != Darwin ]; then
    echo "install-app: skipped (macOS-only: renamex_np, ADR-0001 §9.3)"
    exit 0
fi
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
mkdir -p "$t/bin" "$t/src/crank.app/Contents/MacOS" "$t/dst"
cat > "$t/bin/codesign" <<'STUB'
#!/bin/sh
echo "$*" >> "$CODESIGN_LOG"
case "$*" in *"-d -r-"*) echo 'designated => identifier "io.x" and certificate leaf = H"abc"' >&2;; esac
STUB
chmod +x "$t/bin/codesign"
export PATH="$t/bin:$PATH" CODESIGN_LOG="$t/codesign.log"
bin=$t/dst/crank.app/Contents/MacOS/crank
echo v1 > "$t/src/crank.app/Contents/MacOS/crank"
"$here/install-app.sh" "$t/src/crank.app" "$t/dst/crank.app" abc io.x >/dev/null
[ "$(cat "$bin")" = v1 ] || { echo "fresh install failed" >&2; exit 1; }
# #165.1: both signing calls select the identity by the SHA-1 the requirement check verifies, never by name.
[ "$(grep -c -e '--force --sign abc --identifier' "$t/codesign.log")" = 2 ] || { echo "signing did not use --sign <sha1>" >&2; exit 1; }
if grep -q -e '--sign nix-tcc' "$t/codesign.log"; then echo "signed by name" >&2; exit 1; fi
echo v2 > "$t/src/crank.app/Contents/MacOS/crank"
"$here/install-app.sh" "$t/src/crank.app" "$t/dst/crank.app" abc io.x >/dev/null
[ "$(cat "$bin")" = v2 ] || { echo "swap failed" >&2; exit 1; }
echo v3 > "$t/src/crank.app/Contents/MacOS/crank"
if "$here/install-app.sh" "$t/src/crank.app" "$t/dst/crank.app" WRONG io.x >/dev/null 2>&1; then
    echo "mismatched identity was accepted" >&2; exit 1
fi
[ "$(cat "$bin")" = v2 ] || { echo "failed install damaged the old app" >&2; exit 1; }
[ "$(ls -A "$t/dst")" = crank.app ] || { echo "temp dir left behind" >&2; exit 1; }
echo "install-app: ok"
