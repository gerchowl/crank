#!/usr/bin/env python3
"""Size of crank's surface index for one OSM extract, ADR-0005 §2.6.1 (#523).

Stdlib only. Reads the GeoJSON text sequence that `osm_extract_cost.sh` makes with osmium (the highway ways of an
extract as linestrings), drops the non-ridable highway values of §2.6.1 and computes what crank's index would
store: geometry as zigzag-varint deltas of 1e-7° coordinates, plus a 0.01° grid of (cell, way) pairs at 4 B. It
also reports the tag coverage (surface, tracktype, smoothness) and the node count, which bounds an in-memory
builder (16 B per node: id and coordinates).

Run: python3 docs/adr/models/osm_index_size.py <highways.geojsonseq>
"""
from __future__ import annotations

import collections
import json
import sys

# §2.6.1: highway values crank does not keep
NOT_RIDABLE = {"motorway", "motorway_link", "construction", "proposed", "abandoned", "razed", "planned", "steps",
               "elevator", "platform", "raceway", "bus_guideway", "busway", "corridor", "escape", "via_ferrata"}
WAY_HEADER_B = 4      # node count varint (about 2 B), class byte, flags byte
GRID_PAIR_B = 4       # one (cell, way) entry of the 0.01° grid
NODE_B = 16           # an in-memory builder's node: id (8 B) and coordinates (2 × 4 B)


def varint_len(n):
    """Bytes of the zigzag LEB128 varint of a signed integer n."""
    z = (n << 1) ^ (n >> 63)
    b = 1
    while z >= 0x80:
        z >>= 7
        b += 1
    return b


def measure(path):
    m = collections.Counter()
    highway, surface = collections.Counter(), collections.Counter()
    cells = set()
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.lstrip("\x1e").strip()
            if not line:
                continue
            o = json.loads(line)
            g = o.get("geometry") or {}
            tags = o.get("properties", {})
            h = tags.get("highway")
            if g.get("type") != "LineString" or h is None:
                continue
            if h in NOT_RIDABLE:
                m["dropped"] += 1
                continue
            coords = g["coordinates"]
            m["ways"] += 1
            m["nodes"] += len(coords)
            highway[h] += 1
            for k in ("surface", "tracktype", "smoothness"):
                m[k] += k in tags
            if "surface" in tags:
                surface[tags["surface"]] += 1
            m["geometry_b"] += WAY_HEADER_B
            px = py = 0
            way_cells = set()
            for lon, lat in coords:
                x, y = round(lon * 1e7), round(lat * 1e7)
                m["geometry_b"] += varint_len(x - px) + varint_len(y - py)
                px, py = x, y
                way_cells.add((int(lat * 100), int(lon * 100)))
            cells |= way_cells
            m["pairs"] += len(way_cells)
    return m, highway, surface, len(cells)


def main(argv):
    if len(argv) != 2:
        print(__doc__.strip().splitlines()[-1], file=sys.stderr)
        return 2
    m, highway, surface, n_cells = measure(argv[1])
    ways = m["ways"]
    print(f"ways kept {ways} (dropped {m['dropped']} not ridable), nodes {m['nodes']}, "
          f"{m['nodes'] / ways:.1f} nodes per way")
    print(f"with surface {m['surface']} ({m['surface'] / ways:.0%}), tracktype {m['tracktype']}, "
          f"smoothness {m['smoothness']}")
    grid_b = m["pairs"] * GRID_PAIR_B
    print(f"index: geometry {m['geometry_b'] / 1e6:.1f} MB + grid {grid_b / 1e6:.1f} MB "
          f"({m['pairs']} (cell, way) pairs over {n_cells} cells of 0.01°) = "
          f"{(m['geometry_b'] + grid_b) / 1e6:.1f} MB")
    print(f"in-memory builder: {m['nodes'] * NODE_B / 1e6:.0f} MB of node ids and coordinates before hash overhead")
    print("top highway:", highway.most_common(8))
    print("top surface:", surface.most_common(10))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
