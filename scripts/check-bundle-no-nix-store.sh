#!/usr/bin/env bash
# Fail if any Mach-O in a crank.app bundle refers to /nix/store (#413): a load command (otool -L) or an
# rpath (otool -l) there means a nix garbage collection breaks the installed app.
# Usage: check-bundle-no-nix-store.sh <crank.app>
set -euo pipefail
bundle=${1:?usage: check-bundle-no-nix-store.sh <crank.app>}
checked=0 bad=0
while IFS= read -r -d '' f; do
    # otool exits 0 on any file, so recognise Mach-O by magic (thin 64/32-bit either endian, fat).
    case $(od -An -tx1 -N4 "$f" | tr -d ' \n') in
        cffaedfe|cefaedfe|feedfacf|feedface|cafebabe|bebafeca) ;;
        *) continue ;;
    esac
    checked=$((checked + 1))
    refs=$({ otool -L "$f" | tail -n +2; otool -l "$f" | grep -A2 LC_RPATH; } | grep /nix/store || true)
    if [ -n "$refs" ]; then
        echo "$f references /nix/store:" >&2
        echo "$refs" >&2
        bad=$((bad + 1))
    fi
done < <(find "$bundle" -type f -print0)
[ "$checked" -gt 0 ] || { echo "no Mach-O found in $bundle" >&2; exit 1; }
[ "$bad" -eq 0 ] || { echo "$bad of $checked Mach-O files reference /nix/store" >&2; exit 1; }
echo "$checked Mach-O files, none references /nix/store"
