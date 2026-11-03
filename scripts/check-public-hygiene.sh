#!/bin/sh
# Public-repo hygiene gate (#122, #146): tracked files must not contain
# internal tailnet hostnames or absolute macOS home paths.
# Usage: check-public-hygiene.sh [root]  (default: current directory)
# The patterns use bracket expressions so this file does not match itself;
# nothing is excluded.
set -eu
cd "${1:-.}"
patterns='[.]ts[.]net|/Users[/]'
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  hits=$(git ls-files -z | xargs -0 grep -IncE "$patterns" 2>/dev/null | grep -v ':0$' || true)
else
  hits=$(grep -rIcE "$patterns" . 2>/dev/null | grep -v ':0$' || true)
fi
if [ -n "$hits" ]; then
  echo "public-hygiene: forbidden patterns (internal hostname or /Users path) in:" >&2
  echo "$hits" >&2
  exit 1
fi
echo "public-hygiene: ok"
