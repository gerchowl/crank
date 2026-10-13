"""Check all-feature crate boundaries and the separate production feature graph.

Read all-feature Cargo metadata on stdin and production Cargo metadata from the
file argument. Separating graphs avoids enabling e2e just by inspecting all
features, while still detecting its accidental activation in production.
"""
import json
import sys


def fail(message):
    """Report a boundary violation even when Python assertions are disabled."""
    sys.exit(f"deps-edges: {message}")


def graph(metadata):
    """Index resolved packages and require every scaffold workspace crate."""
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    expected = {f"crank-{name}" for name in (
        "domain", "devices", "fec", "ble", "engine", "store", "coach",
        "intervals", "core", "mcp", "cli", "desktop",
    )}
    workspace = {packages[p]["name"] for p in metadata["workspace_members"]}
    missing = expected - workspace
    if missing:
        fail(f"missing workspace crates: {', '.join(sorted(missing))}")
    if any(p not in nodes for p in metadata["workspace_members"]):
        fail("workspace crate missing from resolved graph")
    return packages, nodes


def reachable(root, packages, nodes):
    """Walk all resolved dependency edges, including transitive dependencies."""
    pending = [p for p in nodes if packages[p]["name"] == root]
    if not pending:
        fail(f"missing root crate: {root}")
    seen = set()
    while pending:
        node = pending.pop()
        if node not in seen:
            seen.add(node)
            pending.extend(nodes[node]["dependencies"])
    return seen


if len(sys.argv) != 2:
    fail("usage: deps-edges.py PRODUCTION_METADATA < ALL_FEATURES_METADATA")
packages, nodes = graph(json.load(sys.stdin))
for root, forbidden in [("crank-domain", "tokio"), ("crank-engine", "crank-ble")]:
    if any(packages[p]["name"] == forbidden for p in reachable(root, packages, nodes)):
        fail(f"{root} must not depend on {forbidden}")

with open(sys.argv[1], encoding="utf-8") as production_file:
    packages, nodes = graph(json.load(production_file))
# #37: point this at crank-app's packaged production graph once assembly lands.
for node in reachable("crank-desktop", packages, nodes):
    if "e2e" in nodes[node]["features"]:
        fail(f"production desktop enables e2e in {packages[node]['name']}")
