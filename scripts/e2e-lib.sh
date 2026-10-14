# Sourced by scripts/e2e.sh and scripts/test-e2e-prune.sh; callers run under `set -euo pipefail`.

# prune_runs <out> <current-run-dir> <keep>: remove all but the <keep> newest `run.*` dirs in <out>,
# never touching <current-run-dir>. Safe with zero matches: a bare `ls | grep | tail` pipeline exits
# non-zero then, which `set -e`/pipefail turns into a silent script abort (#233).
prune_runs() {
  local out=$1 current=$2 keep=$3 dir n=0
  local -a old=()
  while IFS= read -r dir; do
    [ -n "$dir" ] && old+=("$dir")
  done < <(find "$out" -maxdepth 1 -type d -name 'run.*' ! -path "$current" -exec ls -dt {} + 2>/dev/null || true)
  # `${old[@]+...}`: an empty array is unbound under `set -u` in bash 3.2 (macOS).
  for dir in ${old[@]+"${old[@]}"}; do
    n=$((n + 1))
    [ "$n" -le "$keep" ] || rm -rf "$dir"
  done
}
