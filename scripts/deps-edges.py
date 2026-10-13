"""Enforce the ADR-0001 §2 crate boundaries from resolved Cargo metadata."""
import json
import sys

metadata = json.load(sys.stdin)
packages = {p["id"]: p for p in metadata["packages"]}
nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
for name, forbidden in [("crank-domain", "tokio"), ("crank-engine", "crank-ble")]:
    pending = [p["id"] for p in packages.values() if p["name"] == name]
    seen = set()
    while pending:
        node = pending.pop()
        if node in seen:
            continue
        seen.add(node)
        assert packages[node]["name"] != forbidden, f"{name} must not depend on {forbidden}"
        pending.extend(nodes[node]["dependencies"])
for node in nodes.values():
    if packages[node["id"]]["name"] == "crank-desktop":
        assert "e2e" not in node["features"], "production desktop must not enable e2e"
