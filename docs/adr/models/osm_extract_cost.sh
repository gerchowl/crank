#!/bin/bash
# Cost of crank's local OSM surface index for one extract, ADR-0005 §2.6.1 (#523).
# Needs osmium-tool (e.g. `nix shell nixpkgs#osmium-tool`) and python3; macOS `/usr/bin/time -l` for peak memory.
# Usage: docs/adr/models/osm_extract_cost.sh <region.osm.pbf> <work dir>
# The work dir receives the filtered PBF (hw.osm.pbf) and the ways as GeoJSON text (hw.geojsonseq): a few hundred
# MB for a country. The extract itself comes from https://download.geofabrik.de/ (check it against its .md5).
set -euo pipefail
[ $# -eq 2 ] || { echo "usage: $0 <region.osm.pbf> <work dir>" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
pbf=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
mkdir -p "$2" && cd "$2"
osmium --version | head -1
ls -l "$pbf"
echo "== filter: highway ways and the nodes they reference"
/usr/bin/time -l osmium tags-filter -O "$pbf" w/highway -o hw.osm.pbf 2> tf.time
grep -E "real|maximum resident" tf.time; ls -l hw.osm.pbf
echo "== way geometry (linestrings)"
/usr/bin/time -l osmium export -O hw.osm.pbf -f geojsonseq --geometry-types=linestring -o hw.geojsonseq 2> ex.time
grep -E "real|maximum resident" ex.time; ls -l hw.geojsonseq
echo "== index size"
python3 -I "$here/osm_index_size.py" hw.geojsonseq
