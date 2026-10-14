#!/usr/bin/env bash
# prune_runs (scripts/e2e-lib.sh) with zero, one and five prior run dirs under `set -euo pipefail`:
# it must not exit non-zero, must keep the newest ones and the current dir, and must ignore other files (#233).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=e2e-lib.sh
. "$here/e2e-lib.sh"
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT

fail() { echo "e2e-prune: $*" >&2; exit 1; }
# case <name> <number of prior runs> <expected survivors, oldest-first indices>
mk() { # mk <out> <count>: run.1 oldest ... run.<count> newest, plus the current dir and a bystander
  mkdir -p "$1/app" "$1/cur"
  : > "$1/keepme.txt"
  for i in $(seq 1 "$2"); do
    mkdir "$1/run.$i"
    touch -t "2026010100$(printf %02d "$i")" "$1/run.$i"
  done
}
survivors() { (cd "$1" && ls -d run.* 2>/dev/null | sort | tr '\n' ' ') || true; }

for case in "0:" "1:run.1 " "5:run.3 run.4 run.5 "; do
  n=${case%%:*}; want=${case#*:}
  out=$t/case-$n
  mk "$out" "$n"
  prune_runs "$out" "$out/cur" 3 || fail "prune_runs exited non-zero with $n prior runs"
  got=$(survivors "$out")
  [ "$got" = "$want" ] || fail "$n prior runs: kept '$got', want '$want'"
  [ -d "$out/cur" ] && [ -e "$out/keepme.txt" ] && [ -d "$out/app" ] || fail "$n prior runs: removed a non-run entry"
done
# The current dir is never pruned even when it matches run.* and is the oldest.
out=$t/current-is-run
mk "$out" 5
mkdir "$out/run.cur"; touch -t 202001010000 "$out/run.cur"
prune_runs "$out" "$out/run.cur" 3
[ -d "$out/run.cur" ] || fail "removed the current run dir"
[ "$(survivors "$out")" = "run.3 run.4 run.5 run.cur " ] || fail "current-is-run: kept '$(survivors "$out")'"
echo "e2e-prune: ok"
