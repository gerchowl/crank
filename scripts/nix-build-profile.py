#!/usr/bin/env python3
"""Profile a Nix build from its internal-json log (ADR-0006 §1, #580).

Usage:
    set -o pipefail
    nix build --log-format internal-json -v <installable> 2>&1 | scripts/nix-build-profile.py [--json out.json]

Reads the log from stdin, stamps each event with its arrival time and prints:
the wall time, every derivation built (name, wall seconds, how many workspace
crates cargo compiled, checked or documented in it), the number of paths
substituted (by cache) and, per workspace crate, how many times it was compiled
across the whole build. Workspace crates are the `crank-*` packages cargo
reports from a path inside the build tree (`Compiling crank-core v0.1.0
(/build/...)`); dependencies come from the vendor directory and do not count.
"""

import argparse
import collections
import json
import re
import sys
import time

BUILD = 105  # ActivityType::Build
SUBSTITUTE = 108  # ActivityType::Substitute
BUILD_LOG_LINE = 101  # ResultType::BuildLogLine
CARGO = re.compile(r"^\s*(Compiling|Checking|Documenting) (crank-[a-z-]+) v\S+ \(")
DRV = re.compile(r"/nix/store/[a-z0-9]{32}-(.+)\.drv")
ANSI = re.compile(r"\x1b\[[0-9;]*m")


def main() -> int:
    """Read the log on stdin and print the profile."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--json", help="also write the profile as JSON to this path")
    args = parser.parse_args()

    start = time.monotonic()
    builds = {}  # drv path -> {name, start, stop, cargo: Counter}
    activity = {}  # activity id -> drv path (a remote build reports one drv under two activities)
    substituted = collections.Counter()  # cache uri -> paths
    errors = []
    for raw in sys.stdin:
        now = time.monotonic() - start
        if not raw.startswith("@nix "):
            if raw.strip():
                errors.append(raw.rstrip("\n"))
            continue
        try:
            event = json.loads(raw[5:])
        except json.JSONDecodeError:
            continue
        action = event.get("action")
        if action == "start" and event.get("type") == BUILD:
            fields = event.get("fields") or [""]
            match = DRV.search(fields[0])
            activity[event["id"]] = fields[0]
            builds.setdefault(fields[0], {"name": match.group(1) if match else fields[0], "start": now,
                                          "stop": None, "cargo": collections.Counter()})
        elif action == "start" and event.get("type") == SUBSTITUTE:
            fields = event.get("fields") or ["", ""]
            substituted[fields[1] if len(fields) > 1 else ""] += 1
        elif action == "stop" and event.get("id") in activity:
            builds[activity[event["id"]]]["stop"] = now
        elif action == "result" and event.get("type") == BUILD_LOG_LINE and event.get("id") in activity:
            line = ANSI.sub("", str((event.get("fields") or [""])[0]))
            match = CARGO.match(line)
            if match:
                builds[activity[event["id"]]]["cargo"][match.group(2)] += 1
        elif action == "msg" and event.get("level", 9) <= 0:
            errors.append(event.get("msg", ""))
    wall = time.monotonic() - start

    per_crate = collections.Counter()
    rows = []
    for build in builds.values():
        per_crate.update(build["cargo"])
        stop = build["stop"] if build["stop"] is not None else wall
        rows.append({"name": build["name"], "seconds": round(stop - build["start"], 1),
                     "crank_compiles": sum(build["cargo"].values()), "cargo": dict(build["cargo"])})
    rows.sort(key=lambda r: -r["seconds"])

    print(f"wall: {wall:.1f} s; built: {len(rows)} derivations; substituted: {sum(substituted.values())} paths "
          f"{dict(substituted)}")
    for row in rows:
        print(f"{row['seconds']:9.1f} s  {row['name']}  (workspace crates compiled: {row['crank_compiles']})")
    print("compiles per workspace crate:", ", ".join(f"{k} {v}" for k, v in sorted(per_crate.items())) or "none")
    for line in errors[-20:]:
        print("log:", line)
    if args.json:
        with open(args.json, "w", encoding="utf-8") as out:
            json.dump({"wall_seconds": round(wall, 1), "builds": rows, "substituted": dict(substituted),
                       "compiles_per_crate": dict(per_crate)}, out, indent=1)
    return 0


if __name__ == "__main__":
    sys.exit(main())
