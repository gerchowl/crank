#!/usr/bin/env bash
# Install and sign crank.app without ever leaving a window with no app (#165).
# Usage: install-app.sh <built crank.app> <target crank.app> <identity SHA-1> <bundle id>
# The copy is signed and verified at a temp path beside the target, then swapped in with
# renamex_np(RENAME_SWAP), so the target is always either the old or the new complete app.
set -euo pipefail
src=$1 app=$2 sha1=$3 id=$4
mkdir -p "$(dirname "$app")"
tmp=$(mktemp -d "$(dirname "$app")/.crank-install.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
new=$tmp/crank.app
cp -R "$src" "$new"
chmod -R u+w "$new"
# Sign by SHA-1: the identity that signs is the one the requirement check verifies (name could be ambiguous).
codesign --force --sign "$sha1" --identifier "$id.cli" "$new/Contents/MacOS/crank"
codesign --force --sign "$sha1" --identifier "$id" "$new"
codesign --verify --strict --verbose=2 "$new"
want="identifier \"$id\" and certificate leaf = H\"$sha1\""
got=$(codesign -d -r- "$new" 2>&1 | sed -n 's/^designated => //p')
echo "designated => $got"
[ "$got" = "$want" ] || { echo "requirement mismatch, want: $want" >&2; exit 1; }
if [ -e "$app" ]; then
    # Atomic exchange; the previous app lands in $new and is removed with $tmp.
    python3 - "$new" "$app" <<'PY'
import ctypes, sys
libc = ctypes.CDLL(None, use_errno=True)
if libc.renamex_np(sys.argv[1].encode(), sys.argv[2].encode(), 2) != 0:  # RENAME_SWAP
    sys.exit(f"renamex_np swap failed: errno {ctypes.get_errno()}")
PY
else
    mv "$new" "$app"
fi
echo "signed $app"
