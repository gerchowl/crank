#!/usr/bin/env bash
# Exercise scripts/install-app.sh against a temp target with a stub codesign (#165): fresh install,
# atomic swap over an existing app, and a requirement mismatch that must leave the old app untouched.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
mkdir -p "$t/bin" "$t/src/crank.app/Contents/MacOS" "$t/dst"
cat > "$t/bin/codesign" <<'STUB'
#!/bin/sh
case "$*" in *"-d -r-"*) echo 'designated => identifier "io.x" and certificate leaf = H"abc"' >&2;; esac
STUB
chmod +x "$t/bin/codesign"
export PATH="$t/bin:$PATH"
bin=$t/dst/crank.app/Contents/MacOS/crank
echo v1 > "$t/src/crank.app/Contents/MacOS/crank"
"$here/install-app.sh" "$t/src/crank.app" "$t/dst/crank.app" abc io.x >/dev/null
[ "$(cat "$bin")" = v1 ] || { echo "fresh install failed" >&2; exit 1; }
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
