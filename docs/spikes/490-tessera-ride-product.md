# Spike #490: one Tessera `.tsra` per ride

**Question** (#490 and the owner's refinement): could one Tessera product per ride hold the whole ride
(manifest, workout, samples, frames, logs, summary), replace #489's `.fecap.zst`, and still be small,
crash safe, lossless, queryable and light enough to ship in the desktop app?

**Answer: skip for now.** Do not adopt Tessera in the app. #489's zstd-compressed `.fecap` stays the
original copy, and the SQLite store (#158) stays the ride record. Re-evaluate under the conditions of
#491 (the format freeze, small-table cost, binary weight). The reasons are measured below:

- The lossless per-ride product is **83,345 B**, against 64,950 B for the `.fecap` text and 12,210 B for
  zstd-19. That is 6.8 times the #489 baseline. Its frames alone are 45,977 B (3.8 times).
- Tessera has no binary column type, so the raw payload is stored either as an uncompressed blob or in
  bulkier column forms. Each small table block also has a fixed cost of a few KB (6,612 B for a 3-row table).
- No supported write path appends rows during a ride and leaves a readable table after a crash, at
  crank's data rates (below).
- Adding `tessera-io` adds **+33.3 MB** to a stripped release binary (the installed crank-desktop binary of 2026-10-13 is 8.5 MB) and
  156 crate names that are not yet in crank's `Cargo.lock`. It also fails crank's `cargo deny` licence
  and source policy.
- The format is not frozen: there is no v0.1 tag yet, and the README says to keep the original data.

Byte-exact round-trip and querying work well: both pass (sections 4 and 5).

## Setup

- **Tessera** pinned by commit: `vig-os/tessera` `dev` at `a3df683b2574f5257b99c09710b49629c9daa17d`
  (2026-11-03; workspace version `0.1.0-alpha.4`, which already includes #750 and #751). The tag
  `v0.1.0-alpha.4` (`2e9f530`, 2026-10-29) is 74 commits behind it, three of them breaking (`!`): #691,
  #745 and #750. alpha.4 has no `StreamWriter::push_table`, so the spike uses `dev`.
- **Crates used:** `tessera-core` and `tessera-io` only (no `tessera-ingest`, no `cloud`), with crank's
  `crank-fec` (frame decoding) and `crank-domain` (the real DSL parser and compiler) as path dependencies.
  The scratch crate lives outside the repo (`/private/tmp/claude-502/spike-490/`). Its source is in the
  [appendix](#appendix-a-spike-code).
- **Input:** the owner's ride capture of 2026-11-03: 1,261 frames (948 rx, 313 tx, all 13 B) over
  197.7 s, 64,950 B of text. Every number here is derived from it; the capture itself (which holds the
  trainer serial in page 81) is not committed.
- **Toolchain:** rustc 1.96.0 (crank's pin) on aarch64-darwin, `CARGO_BUILD_JOBS=2`; zstd 1.5.7; DuckDB
  v1.5.5 with its `vortex` extension (`9a8eb67`).

## 1. The per-ride product

One `WriteSession` per ride, one block per #490 row, sealed into one `.tsra`:

| #490 block | Tessera form | content in the spike |
|---|---|---|
| manifest | header `extra["crank.manifest"]`, sealed into `manifest.json` | ride id, start/end, app version and commit, FTP, devices as redacted ids (`trainer-1`), `redacted` flag, the `.fecap` lines that are not records (header comment) with their row positions. Content hash and seal are Tessera's own (`content_hash`, `manifest_hash`). |
| workout | blobs `workout/dsl` (`text/plain`) and `workout/compiled` (JSON of `CompiledWorkout`), table `workout/steps` | "Sensor check (3 min)" parsed and compiled by `crank-domain` at FTP 250 W |
| samples | table `samples`, 1 Hz | `t_s, target_w, power_w, cadence_rpm, hr_bpm, speed_mmps, mode, paused, block`, synthesised from the capture: mean page-25 power per second, latest page-16/49 values. `mode` and `paused` are synthetic. |
| frames | table `frames` + blob `frames_raw` | file order, `t_us, dir, len, msg_type, channel, page, power_w, cadence_rpm, speed_mmps, hr_bpm, event_count, target_qw` (nullable where the page has no such field), decoded by `crank_fec::decode`; the raw payload in one of four layouts (below) |
| logs | table `logs` | 5 synthetic vigil records: `time_unix_nano, severity, target, body, span, trace_id, span_id` |
| summary | header `extra["crank.summary"]` | duration, NP, IF, TSS (from the samples), compliance and laps as placeholders |

**Answer to "several table blocks with different schemas plus small blobs?":** yes. One product held
four tables with four schemas and three blobs. Block names may be hierarchical (`workout/steps`), and a
name cannot be both a leaf and a parent (ADR-0063).

**No binary column.** `ColumnData` has integer, float, bool and `str` types (`tessera-io/src/table.rs`),
but nothing for bytes. The raw frame payload therefore needs one of these layouts:

- `lenblob`: a `len` column plus every payload concatenated in a `Blob` block. A blob is stored
  verbatim with no codec.
- `hex`: an upper-case hex `str` column.
- `cols13`: 13 `u1` columns `b00..b12`. This works only because FE-C frames are fixed at 13 B; CPS
  notes and tagged comments are not.
- `none`: decoded columns only. This is lossy and is shown only for comparison.

## 2. Size

`ride490 build <capture> <out> <stage> <layout> 0` (exit 0 for each layout), then
`ride490 sizes <out>` (exit 0). A `.tsra` is a STORED zip, so each block's size is its size on disk.
The baselines were checked again: `wc -c` gives 64,950, and `zstd -19 -q -c | wc -c` gives 12,210 (both exit 0).

| block | lenblob | hex | cols13 | none (lossy) |
|---|---:|---:|---:|---:|
| `frames` | 29,584 | 48,348 | 53,896 | 29,584 |
| `frames_raw` (blob) | 16,393 | | | |
| **frames total** | **45,977** | **48,348** | **53,896** | 29,584 |
| `samples` (198 rows) | 13,020 | 13,020 | 13,020 | 13,020 |
| `logs` (5 rows) | 8,940 | 8,940 | 8,940 | 8,940 |
| `workout/steps` (3 rows) | 6,612 | 6,612 | 6,612 | 6,612 |
| `workout/compiled` + `workout/dsl` | 529 | 529 | 529 | 529 |
| manifest + zip | 8,267 | 7,958 | 9,393 | 7,833 |
| **whole `.tsra`** | **83,345** | **85,407** | **92,390** | **66,518** |
| vs `.fecap` text (64,950) | 1.28× | 1.31× | 1.42× | 1.02× |
| vs `.fecap` zstd-19 (12,210) | 6.8× | 7.0× | 7.6× | 5.4× |

Notes:

- Even the decoded columns alone (29,584 B) are 2.4 times the zstd-19 text, which is lossless.
- The fixed cost of each table block (a Vortex file with its footer, layout and statistics) dominates at
  ride scale: 6,612 B for 3 rows by 5 columns. Data this redundant shows in `zstd -19` of the finished
  product: 83,345 B compress to 22,595 B (exit 0).
- `frames_raw` is 1,261 × 13 = 16,393 B exactly, because a blob is not compressed.
- **A one-hour ride** (synthetic: the capture looped 18 times with shifted times, 22,698 frames, 1,196,033 B
  of text, 99,153 B as zstd-19) gives a lossless `lenblob` product of 564,431 B, of which frames are
  218,744 B and `frames_raw` 295,074 B (`build` and `sizes` exit 0). That is 5.7 times the zstd text. The loop
  repeats data, which flatters zstd's long-range matching, so treat this ratio as an upper bound. The
  ordering is still clear.
- The redacted variant (`build … lenblob 1`, exit 0) is 83,344 B.

## 3. Appending during a ride, and crash safety

**What `dev` supports.** There are three write paths, and none of them fits a live ride:

- **`WriteSession::append_block`** commits one *encoded* block durably: it fsyncs the fragment and its
  directories, then writes a journal line (FEATURE-MATRIX §E). A committed block is immutable, so rows
  cannot be appended to it. Appending during a ride therefore means one new block per chunk
  (`samples_chunk/c00000`, …).
- **`StreamWriter::push_table`** (#750, #751) is the supported way to write a *large* table. It needs
  `spec.rows` up front ("the input runs past `spec.rows` … ends short of it" is an error) and partitions
  above `BLOCK_ROWS` = 2²² = 4,194,304 rows into `<table>/NNNN` leaves. Its heap bound is
  `min(W, n) · p_block + s` with `s = 2 · ROWS_PER_GROUP · r_mem + 4 MiB` (for example, 7 MiB for 24-byte
  rows). `ROWS_PER_GROUP` is 2¹⁶. It is a ride-end path: the row count is not known while riding.
- **`table_job_streaming`** is deprecated by #750 and refuses tables above `BLOCK_ROWS`.
  **`TableStreamWriter`** (the DAQ accumulator) takes batches of any size but spills to durable fragments
  only per *full* 65,536-row group. A one-hour ride has about 23,000 frames, so the whole ride would stay
  in RAM, and `TableStreamWriter` has no `recover`.

A logical table over chunk blocks is no way out either: the `<table>/NNNN` partition (ADR-0064) puts
`BLOCK_ROWS` rows in every leaf but the last, and the total must be known to name them. Small chunks
therefore stay separate tables, which every reader would have to union.

**Crash probe (measured).** `ride490 crash-child <stage>` appends one 60-row, 2-column table block
every 50 ms through `append_block`. The probe killed it with `kill -9` after 1.3 s (child exit 137), then ran
`ride490 recover <stage> <out>`, which calls `WriteSession::recover`, then seals and reads back every block
with digest checks. It ran twice:

| run | last commit the child printed | journal lines | recovered | sealed and verified | `recover` exit |
|---|---:|---:|---:|---:|---:|
| a | 17 | 18 | 18 | 18 | 0 |
| b | 16 | 17 | 17 | 17 | 0 |

So the journal is crash safe to the last committed block. **Before** `recover`, nothing is a `.tsra`, but
each committed fragment under `<stage>/blocks/` is a plain Vortex file that DuckDB reads directly
(`read_vortex('…/samples_chunk/c00000')` gave 60 rows, exit 0). The cost is size: these chunks
averaged **3,291 B for 60 rows × 2 columns** (360 B of raw data). A ride written this way, per minute
and per table, would be several times larger again than the single-block product.

**Answer:** a crash-safe journal during the ride, turned into a product at ride end, is the only shape
that works. crank already has that journal: the SQLite store's `ride_chunk` rows (#158) and the
per-connection `.fecap`. A Tessera product would be an *export* built from them in one go
(`WriteSession` + `append_block` per block, or `push_table` with the known row count).

## 4. Round-trip to `.fecap`

`ride490 fecap <product> <out.fecap>`, then `cmp <capture> <out.fecap>`:

| layout | `fecap` exit | `cmp` exit |
|---|---:|---:|
| lenblob | 0 | 0 (byte for byte) |
| hex | 0 | 0 |
| cols13 | 0 | 0 |
| none | 1 ("no raw payload in this product") | — |
| one-hour `lenblob` | 0 | 0 |

Rows keep file order. Note that `.fecap` time is not monotonic: tx and rx are written by different paths,
so an rx at 2.62 s follows a tx at 2.81 s. A line becomes a row only if it re-renders byte for byte.
Every other line (the header comment here; `# cps`, notes and outcome lines in other captures) is kept
verbatim in the manifest with its row position, so the round-trip is lossless for any input.

The redacted product (`build … lenblob 1`) round-trips (exit 0) to exactly what `hil_fec redact`'s rule
produces: the page-81 serial is set to `FF FF FF FF` and the checksum is fixed (`cmp` against an
independent Python implementation of `redact_frame`, exit 0). The result differs from the original
(`cmp -s` exit 1).

## 5. Query

The `.tsra` is a STORED zip and each table block is a plain Vortex file, so DuckDB needs no Tessera code
at all:

```sh
unzip -o -q ride.tsra -d unz            # exit 0
cd unz && duckdb -box -c "LOAD vortex;
SELECT st.block, st.start_s, st.end_s, st.power_from_w AS plan_w,
       count(s.power_w) AS n_s, round(avg(s.power_w),1) AS mean_power_w,
       round(avg(s.target_w),1) AS mean_target_w
FROM read_vortex('blocks/workout/steps') st
JOIN read_vortex('blocks/samples') s ON s.t_s >= st.start_s AND s.t_s < st.end_s
GROUP BY ALL ORDER BY st.block;"        # exit 0 (after a one-time `INSTALL vortex;`)
```

| block | start_s | end_s | plan_w | n_s | mean_power_w | mean_target_w |
|---|---:|---:|---:|---:|---:|---:|
| 0 | 0 | 60 | 125 | 58 | 53.1 | 68.7 |
| 1 | 60 | 120 | 200 | 60 | 138.8 | 141.0 |
| 2 | 120 | 180 | 100 | 60 | 132.7 | 128.3 |

The capture's own targets come from a different ride plan (an ERG ramp), so the workout alignment is
synthetic. The point is the join. Column types arrive intact (`DESCRIBE` shows `uinteger`, `usmallint`,
`utinyint` and `boolean`, with NULLs from the validity masks). A frames query, `SELECT page, dir,
count(*), avg(power_w) FROM read_vortex('blocks/frames') GROUP BY ALL`, gives page 25 rx: 581 rows,
mean 111.5 W (exit 0). In-tree alternatives are `tessera read <file> <table> --format csv` and
`tessera sql` (DataFusion, behind a feature). Neither was built: `tessera-cli` depends on
`tessera-ingest`, which links libhdf5.

## 6. Dependency footprint (`tessera-core` + `tessera-io`, default features)

A minimal crate (`footprint490`: one `WriteSession`, one table and one blob, seal, read back) was
compared against a baseline with the same shape that uses only `serde_json`. Both were release builds
with rustc 1.96.0 and `CARGO_BUILD_JOBS=2`, each in a fresh target dir.

| measure | value | command, exit |
|---|---|---|
| normal dependencies | 271 crates (baseline: 5) | `cargo tree -e normal --prefix none \| sort -u`, 0 |
| with build dependencies | 291 | `cargo tree -e normal,build …`, 0 |
| not yet in crank's `Cargo.lock` (by name) | 156 (crank's lock has 525 packages) | lock comparison script, 0 |
| release binary, stripped | 33,700,912 B vs 408,112 B, so **+33.3 MB** (crank-desktop is 8,543,248 B) | `strip -o`, 0 |
| release binary, unstripped | 40,664,528 B vs 507,024 B | `ls -l`, 0 |
| cold release build | 143 s (baseline 3 s); the spike crate with crank's crates, 163 s | `cargo +1.96.0 build --release`, 0 |
| libhdf5 | **not pulled in** (only `tessera-ingest` links it) | `cargo tree`, 0 |
| other heavy parts that come in anyway | the whole Vortex 0.75 stack (27 `vortex-*` crates, arrow 58), **zarrs + pcodec + zstd-sys** (the array backend: `tessera-io` has no feature to drop it), tokio, and `bindgen`/`clang-sys` as a build dependency (via `custom-labels` ← `vortex-io`) | `cargo tree -i …`, 0 |
| dynamic libraries | adds `libiconv.2.dylib` (a Nix store path in a dev build, the case #413's bundle check covers) and CoreFoundation | `otool -L`, 0 |
| MSRV | builds on 1.96.0; the highest `rust-version` in the tree is 1.91 (vortex-*); Tessera declares none (its toolchain is `stable`) | `cargo metadata`, 0 |
| `cargo deny` with crank's `deny.toml` | **fails**: `enum-iterator` and `enum-iterator-derive` (via `vortex-array`) are `0BSD`, which is not on the allow list; the tessera git source is not in `allow-git` | `cargo deny --manifest-path … --config deny.toml check licenses sources` in `nix develop`, exit 12 |

Both deny failures can be fixed by policy (adding `0BSD`, and adding the tessera repo to `allow-git`
until crates.io), but each needs an owner decision. The binary weight cannot be fixed from crank's side.

## 7. Format stability

- The README (`dev`) says: "Pre-1.0 — the on-disk format is not yet frozen … the byte format may still
  change before the v0.1 freeze. **Keep your original data**."
- The freeze plan is FEATURE-MATRIX §H. **v0.1 is the format freeze**: core and io, conformance corpus,
  §C and §D gates, zip layout, CLI. v1.0 is "spec frozen + 12-mo zero-breaking". The ROADMAP says
  "v0.1 freezes the wire format". `release-plz.toml` only *holds* release PRs: a release is an operator's
  tag, and there is no date. The open GitHub milestones are `0.1.0-alpha.2` and `0.1.0-beta`
  (30 open issues); there is no `0.1.0` milestone.
- **Churn is current:** 3 breaking (`!`) changes in the 74 commits between alpha.4 and `dev`, two of
  them on 2026-11-03 (#745: multi-block tables as `<table>/NNNN`; #750). ADR-0064 (logical tables) is
  still ◑: the ksb RSS proof (#715) and the consumer notice (#716) are open. Vortex itself is 0.x.
- **Risk of adopting before v0.1:** a `.tsra` written today may need re-encoding or migration. As the
  only copy of a ride, it could become unreadable to a later Tessera. That rules it out as the
  *original* for #489. As a derived export, the risk is only a re-export.

## 8. Recommendation

**Skip for now; re-evaluate under #491.** Measured against #490's goals, size is worse (6.8 times the
zstd baseline), there is no live append at crank's data rates, the binary weight is +33.3 MB, it fails
crank's deny policy, and the format is not frozen. The parts that work (byte-exact round-trip, multiple
schemas in one sealed product, a provenance seal, and DuckDB reading the blocks with no Tessera code)
do not outweigh these.

What stays: #489's `.fecap.zst` is the original raw capture, and SQLite (#158) is the ride record and
History's source.

**The design shape if it is adopted later** (after #491's conditions hold):

- **An export, never the journal.** The ride is recorded as today: `ride_chunk` rows in SQLite and the
  per-connection `.fecap`, both crash safe. At ride end (or on demand, for example "Export ride"), one
  `WriteSession` builds the product from them in one pass: one block per #490 row, using `push_table` once
  the row count is known. A crash mid-ride loses no Tessera data, because none exists yet. The export is
  simply re-run.
- **SQLite stays the index.** History, summaries and queries read SQLite. The `.tsra` is the archival and
  sharing format, with its seal as tamper evidence (useful for HIL evidence). SQLite could record the
  product's `content_hash` against the ride id.
- **Retention (#489)** applies per block class, not to the whole product. Summary, samples, workout and
  logs are small and kept forever; frames are the heavy part to prune. A sealed product cannot drop a
  block in place (that changes `content_hash`), so pruning means re-exporting without `frames` and
  recording `derived_from` provenance. Alternatively, frames go in a separate product per ride that is
  deleted whole.
- **Privacy:** raw payload columns are marked `Sensitivity::Identifying` (Tessera's column field; the spike
  does this); the manifest carries only redacted device ids, with the mapping kept local. A shareable
  variant is a *derived* product with page-81 serials masked (the spike's `redacted=1` path, matching
  `hil_fec redact`) and its own hash. Under ADR-0003, neither variant is ever attached to a bug report.

**Asks for Tessera that would change the answer** (not filed upstream; for the owner): a compressed
binary/bytes column dtype, a lower fixed cost per small table block, a table-only `tessera-io` feature
without zarrs, pcodec and bindgen, and a durable journal for row appends below one row-group.

## Appendix: commands and exit codes

`B=$CARGO_TARGET_DIR/release/ride490`, `C=<the capture>`.

| step | command | exit |
|---|---|---:|
| build spike | `heavy cargo +1.96.0 build --release` (163 s cold) | 0 |
| products | `$B build $C ride-<layout>.tsra st-<layout> <layout> 0` for lenblob, hex, cols13, none | 0 each |
| sizes | `$B sizes ride-<layout>.tsra` | 0 each |
| baselines | `wc -c < $C`; `zstd -19 -q -c $C \| wc -c` (zstd from `nix shell nixpkgs#zstd`) | 0 |
| round-trip | `$B fecap ride-<layout>.tsra back.fecap && cmp $C back.fecap` | 0 / 0 (none: 1) |
| redacted | `$B build $C red.tsra st-red lenblob 1`; `$B fecap red.tsra back-red.fecap`; `cmp py-red.fecap back-red.fecap` | 0 / 0 / 0 |
| one hour | loop script → `hour.fecap`; `$B build hour.fecap hour-lenblob.tsra … lenblob 0`; `sizes`; `fecap`; `cmp` | 0 each |
| crash | `crash.sh a`, `crash.sh b`: start `crash-child`, `kill -9` after 1.3 s, `$B recover <stage> <out>` | child 137, recover 0 |
| query | `unzip -o -q ride-lenblob.tsra -d unz`; `duckdb -box -c "LOAD vortex; …"` | 0 / 0 |
| footprint | `heavy build-fp.sh` (baseline + footprint release builds, `strip`, `otool -L`) | 0 |
| deps | `cargo +1.96.0 tree -e normal --prefix none`; `cargo +1.96.0 tree -i clang-sys -e normal,build`; `cargo +1.96.0 metadata` | 0 |
| licences | `nix develop -c cargo deny --manifest-path footprint/Cargo.toml --config deny.toml check licenses sources` | 12 |

## Appendix A: spike code

`ride490/Cargo.toml` (the path dependencies point at a crank checkout):

```toml
[package]
name = "ride490"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
tessera-core = { git = "https://github.com/vig-os/tessera", rev = "a3df683b2574f5257b99c09710b49629c9daa17d" }
tessera-io = { git = "https://github.com/vig-os/tessera", rev = "a3df683b2574f5257b99c09710b49629c9daa17d" }
crank-fec = { path = "<crank checkout>/crates/crank-fec" }
crank-domain = { path = "<crank checkout>/crates/crank-domain" }
serde_json = "1"

[workspace]
```

`ride490/src/main.rs`:

```rust
//! Spike #490: one `.tsra` per ride. Scratch code, not product code.
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::Path;

use crank_domain::workout::{CompileOptions, Workout, compile_workout, parse_dsl};
use crank_fec::{Page, decode};
use serde_json::{Value, json};
use tessera_core::block::table::{Column, TableSpec};
use tessera_core::schema::Sensitivity;
use tessera_io::table::{self, ColumnData, TableData};
use tessera_io::{Reader, WriteSession, blob_block};

type R<T> = Result<T, Box<dyn Error>>;

const TS: &str = "2026-11-03T00:00:00Z"; // synthetic: no real ride time in the product
const DSL: &str = "\"Warm-up\" 1m 50%, \"Work\" 1m 80%, \"Cool-down\" 1m 40%";
const FTP: f64 = 250.0;

/// One line of a `.fecap`: a canonical record (stored as a row) or anything else (kept verbatim).
enum Line {
    Rec { t_us: u64, rx: bool, bytes: Vec<u8> },
    Verbatim(String),
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

fn render(t_us: u64, rx: bool, b: &[u8]) -> String {
    format!("{t_us} {} {}", if rx { "rx" } else { "tx" }, hex(b))
}

fn parse(text: &str) -> Vec<Line> {
    text.split_inclusive('\n')
        .map(|raw| {
            let line = raw.strip_suffix('\n').unwrap_or(raw);
            let mut f = line.split(' ');
            let rec = (|| {
                let t_us: u64 = f.next()?.parse().ok()?;
                let rx = match f.next()? {
                    "rx" => true,
                    "tx" => false,
                    _ => return None,
                };
                let bytes: Vec<u8> =
                    f.map(|h| u8::from_str_radix(h, 16).ok()).collect::<Option<_>>()?;
                // only lines that re-render byte for byte become rows; the rest stay verbatim
                (render(t_us, rx, &bytes) == line && raw.ends_with('\n'))
                    .then_some(Line::Rec { t_us, rx, bytes })
            })();
            rec.unwrap_or_else(|| Line::Verbatim(raw.to_string()))
        })
        .collect()
}

fn redact(b: &[u8]) -> Vec<u8> {
    let mut o = b.to_vec();
    if o.len() == 13 && o[0] == 0xa4 && o[4] == 0x51 {
        o[8..12].fill(0xff);
        o[12] = o[..12].iter().fold(0, |x, v| x ^ v);
    }
    o
}

fn col(name: &str, dtype: &str) -> Column {
    Column::new(name, dtype)
}

fn nul<T: Default + Clone>(v: Vec<Option<T>>, wrap: fn(Vec<T>) -> ColumnData) -> ColumnData {
    let validity = v.iter().map(Option::is_some).collect();
    ColumnData::Nullable {
        values: Box::new(wrap(v.into_iter().map(Option::unwrap_or_default).collect())),
        validity,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum RawMode {
    /// `len` column + all payload bytes concatenated in a `frames_raw` blob block.
    LenBlob,
    /// raw payload as an upper-case hex `str` column.
    Hex,
    /// raw payload as 13 `u1` columns `b00..b12` (FE-C frames are always 13 bytes).
    Cols13,
    /// no raw payload (decoded columns only; NOT lossless) - for comparison.
    None,
}

struct Frames {
    spec: TableSpec,
    data: TableData,
    blob: Option<Vec<u8>>,
    verbatim: Vec<Value>,
}

fn frames_table(lines: &[Line], mode: RawMode, redacted: bool) -> R<Frames> {
    let (mut t, mut dir, mut len, mut mt, mut ch, mut page) =
        (vec![], vec![], vec![], vec![], vec![], vec![]);
    let (mut pw, mut cad, mut spd, mut hr, mut ev, mut tgt) =
        (vec![], vec![], vec![], vec![], vec![], vec![]);
    let mut raw_hex = vec![];
    let mut blob = vec![];
    let mut cols13: Vec<Vec<u8>> = vec![vec![]; 13];
    let mut verbatim = vec![];
    let mut row = 0u64;
    for l in lines {
        let (t_us, rx, bytes) = match l {
            Line::Verbatim(s) => {
                verbatim.push(json!({"before_row": row, "text": s}));
                continue;
            }
            Line::Rec { t_us, rx, bytes } => (*t_us, *rx, bytes),
        };
        let bytes = if redacted { redact(bytes) } else { bytes.clone() };
        row += 1;
        t.push(t_us);
        dir.push(u8::from(!rx));
        len.push(u8::try_from(bytes.len())?);
        let fr = decode(&bytes).ok();
        mt.push(fr.map(|f| f.msg_type as u8));
        ch.push(fr.map(|f| f.channel));
        page.push((bytes.len() > 4).then(|| bytes[4]));
        let (mut p, mut c, mut s, mut h, mut e, mut g) = (None, None, None, None, None, None);
        match fr.map(|f| f.page) {
            Some(Page::Trainer(d)) => {
                p = d.power_w;
                c = d.cadence_rpm;
                e = Some(d.event_count);
            }
            Some(Page::GeneralFe(d)) => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    s = d.speed_mps.map(|v| (v * 1000.0).round() as u16);
                }
                h = d.heart_rate_bpm;
            }
            Some(Page::TargetPower { watts }) => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    g = Some((watts * 4.0).round() as u16);
                }
            }
            _ => {}
        }
        pw.push(p);
        cad.push(c);
        spd.push(s);
        hr.push(h);
        ev.push(e);
        tgt.push(g);
        match mode {
            RawMode::LenBlob => blob.extend_from_slice(&bytes),
            RawMode::Hex => raw_hex.push(hex(&bytes)),
            RawMode::Cols13 => {
                if bytes.len() != 13 {
                    return Err("Cols13 needs 13-byte frames".into());
                }
                for (i, b) in bytes.iter().enumerate() {
                    cols13[i].push(*b);
                }
            }
            RawMode::None => {}
        }
    }
    let sens = |c: Column| {
        c.with_sensitivity(if redacted { Sensitivity::Public } else { Sensitivity::Identifying })
    };
    let mut columns = vec![
        col("t_us", "u8").with_unit("us").with_description("elapsed since capture start"),
        col("dir", "u1").with_description("0 rx (trainer to host), 1 tx"),
        col("len", "u1").with_unit("By"),
        col("msg_type", "u1").nullable(),
        col("channel", "u1").nullable(),
        col("page", "u1").nullable(),
        col("power_w", "u2").with_unit("W").nullable(),
        col("cadence_rpm", "u1").with_unit("/min").nullable(),
        col("speed_mmps", "u2").with_unit("mm/s").nullable(),
        col("hr_bpm", "u1").with_unit("/min").nullable(),
        col("event_count", "u1").nullable(),
        col("target_qw", "u2").with_unit("W").with_scale(0.25).nullable(),
    ];
    let mut data: TableData = vec![
        ("t_us".into(), ColumnData::U64(t)),
        ("dir".into(), ColumnData::U8(dir)),
        ("len".into(), ColumnData::U8(len)),
        ("msg_type".into(), nul(mt, ColumnData::U8)),
        ("channel".into(), nul(ch, ColumnData::U8)),
        ("page".into(), nul(page, ColumnData::U8)),
        ("power_w".into(), nul(pw, ColumnData::U16)),
        ("cadence_rpm".into(), nul(cad, ColumnData::U8)),
        ("speed_mmps".into(), nul(spd, ColumnData::U16)),
        ("hr_bpm".into(), nul(hr, ColumnData::U8)),
        ("event_count".into(), nul(ev, ColumnData::U8)),
        ("target_qw".into(), nul(tgt, ColumnData::U16)),
    ];
    match mode {
        RawMode::Hex => {
            columns.push(sens(col("raw_hex", "str")));
            data.push(("raw_hex".into(), ColumnData::Utf8(raw_hex)));
        }
        RawMode::Cols13 => {
            for (i, c) in cols13.into_iter().enumerate() {
                let n = format!("b{i:02}");
                columns.push(sens(col(&n, "u1")));
                data.push((n, ColumnData::U8(c)));
            }
        }
        RawMode::LenBlob | RawMode::None => {}
    }
    Ok(Frames {
        spec: TableSpec { columns, rows: row, row_index: None, partition: None },
        data,
        blob: (mode == RawMode::LenBlob).then_some(blob),
        verbatim,
    })
}

struct Ride {
    frames: Frames,
    samples: (TableSpec, TableData),
    steps: (TableSpec, TableData),
    logs: (TableSpec, TableData),
    compiled: Value,
    summary: Value,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
fn ride(lines: &[Line], mode: RawMode, redacted: bool) -> R<Ride> {
    let frames = frames_table(lines, mode, redacted)?;
    // workout: the real DSL parser and compiler
    let mut n = 0;
    let mut ids = || {
        n += 1;
        format!("n{n}")
    };
    let nodes = parse_dsl(DSL, FTP, &mut ids).map_err(|e| e.message)?;
    let w = Workout { id: "builtin-sensor-check".into(), name: "Sensor check (3 min)".into(), ftp: FTP, nodes };
    let compiled = compile_workout(&w, &CompileOptions::default()).map_err(|e| e.to_string())?;
    let (mut s_idx, mut s_start, mut s_end, mut s_from, mut s_to) = (vec![], vec![], vec![], vec![], vec![]);
    for (i, b) in compiled.blocks.iter().enumerate() {
        s_idx.push(i as u16);
        s_start.push(b.start_s as f32);
        s_end.push((b.start_s + b.dur_s) as f32);
        s_from.push(b.power_from as f32);
        s_to.push(b.power_to as f32);
    }
    let steps = (
        TableSpec {
            columns: vec![
                col("block", "u2"),
                col("start_s", "f4").with_unit("s"),
                col("end_s", "f4").with_unit("s"),
                col("power_from_w", "f4").with_unit("W"),
                col("power_to_w", "f4").with_unit("W"),
            ],
            rows: s_idx.len() as u64,
            row_index: None,
            partition: None,
        },
        vec![
            ("block".into(), ColumnData::U16(s_idx)),
            ("start_s".into(), ColumnData::F32(s_start)),
            ("end_s".into(), ColumnData::F32(s_end)),
            ("power_from_w".into(), ColumnData::F32(s_from)),
            ("power_to_w".into(), ColumnData::F32(s_to)),
        ],
    );
    // samples: 1 Hz, synthesised from the capture (mean page-25 power per second, latest others)
    let mut per_s: BTreeMap<u64, (Vec<u16>, Option<u8>, Option<u16>, Option<u8>, Option<u16>)> =
        BTreeMap::new();
    let mut recs: Vec<(u64, bool, &Vec<u8>)> = lines
        .iter()
        .filter_map(|l| match l {
            Line::Rec { t_us, rx, bytes } => Some((*t_us, *rx, bytes)),
            Line::Verbatim(_) => None,
        })
        .collect();
    recs.sort_by_key(|r| r.0);
    let last_s = recs.last().map_or(0, |r| r.0 / 1_000_000);
    for (t, _, b) in &recs {
        let e = per_s.entry(t / 1_000_000).or_default();
        match decode(b).map(|f| f.page) {
            Ok(Page::Trainer(d)) => {
                if let Some(p) = d.power_w {
                    e.0.push(p);
                }
                e.1 = d.cadence_rpm.or(e.1);
            }
            Ok(Page::GeneralFe(d)) => {
                e.2 = d.speed_mps.map(|v| (v * 1000.0).round() as u16).or(e.2);
                e.3 = d.heart_rate_bpm.or(e.3);
            }
            Ok(Page::TargetPower { watts }) => e.4 = Some(watts.round() as u16),
            _ => {}
        }
    }
    let (mut st, mut tg, mut pw, mut cd, mut hr, mut sp, mut mode_c, mut paused, mut blk) =
        (vec![], vec![], vec![], vec![], vec![], vec![], vec![], vec![], vec![]);
    let (mut c_cad, mut c_spd, mut c_hr, mut c_tgt) = (None, None, None, None);
    for s in 0..=last_s {
        let e = per_s.remove(&s).unwrap_or_default();
        c_cad = e.1.or(c_cad);
        c_spd = e.2.or(c_spd);
        c_hr = e.3.or(c_hr);
        c_tgt = e.4.or(c_tgt);
        let p = (!e.0.is_empty())
            .then(|| (e.0.iter().map(|&x| u32::from(x)).sum::<u32>() / e.0.len() as u32) as u16);
        st.push(s as u32);
        tg.push(c_tgt);
        pw.push(p);
        cd.push(c_cad);
        hr.push(c_hr);
        sp.push(c_spd);
        mode_c.push(u8::from(c_tgt.is_some())); // 0 free ride, 1 ERG (synthetic coding)
        paused.push(false);
        blk.push(
            compiled
                .blocks
                .iter()
                .position(|b| (s as f64) >= b.start_s && (s as f64) < b.start_s + b.dur_s)
                .map(|i| i as u16),
        );
    }
    let rows = st.len() as u64;
    let pw_vals: Vec<f64> = pw.iter().map(|p| f64::from(p.unwrap_or(0))).collect();
    let samples = (
        TableSpec {
            columns: vec![
                col("t_s", "u4").with_unit("s"),
                col("target_w", "u2").with_unit("W").nullable(),
                col("power_w", "u2").with_unit("W").nullable(),
                col("cadence_rpm", "u1").with_unit("/min").nullable(),
                col("hr_bpm", "u1").with_unit("/min").nullable(),
                col("speed_mmps", "u2").with_unit("mm/s").nullable(),
                col("mode", "u1"),
                col("paused", "b1"),
                col("block", "u2").nullable(),
            ],
            rows,
            row_index: None,
            partition: None,
        },
        vec![
            ("t_s".into(), ColumnData::U32(st)),
            ("target_w".into(), nul(tg, ColumnData::U16)),
            ("power_w".into(), nul(pw, ColumnData::U16)),
            ("cadence_rpm".into(), nul(cd, ColumnData::U8)),
            ("hr_bpm".into(), nul(hr, ColumnData::U8)),
            ("speed_mmps".into(), nul(sp, ColumnData::U16)),
            ("mode".into(), ColumnData::U8(mode_c)),
            ("paused".into(), ColumnData::Bool(paused)),
            ("block".into(), nul(blk, ColumnData::U16)),
        ],
    );
    // logs: a few synthetic vigil (OTLP) records of the ride span
    let msgs = [
        (9, "crank_core", "ride started", "ride"),
        (9, "crank_ble", "trainer connected", "device"),
        (13, "crank_ble", "reconnecting trainer", "device"),
        (9, "crank_ble", "trainer reconnected", "device"),
        (9, "crank_core", "ride ended", "ride"),
    ];
    let k = msgs.len();
    let logs = (
        TableSpec {
            columns: vec![
                col("time_unix_nano", "u8").with_unit("ns"),
                col("severity", "u1"),
                col("target", "str"),
                col("body", "str"),
                col("span", "str"),
                col("trace_id", "str"),
                col("span_id", "str"),
            ],
            rows: k as u64,
            row_index: None,
            partition: None,
        },
        vec![
            (
                "time_unix_nano".into(),
                ColumnData::U64((0..k as u64).map(|i| 1_793_664_000_000_000_000 + i * 40_000_000_000).collect()),
            ),
            ("severity".into(), ColumnData::U8(msgs.iter().map(|m| m.0).collect())),
            ("target".into(), ColumnData::Utf8(msgs.iter().map(|m| m.1.into()).collect())),
            ("body".into(), ColumnData::Utf8(msgs.iter().map(|m| m.2.into()).collect())),
            ("span".into(), ColumnData::Utf8(msgs.iter().map(|m| m.3.into()).collect())),
            ("trace_id".into(), ColumnData::Utf8(vec!["5b8efff798038103d269b633813fc60c".into(); k])),
            ("span_id".into(), ColumnData::Utf8((0..k).map(|i| format!("eee19b7ec3c1b17{i}")).collect())),
        ],
    );
    // summary: NP (30 s rolling mean, 4th power), IF, TSS, compliance per block
    let roll: Vec<f64> = pw_vals
        .windows(30.min(pw_vals.len().max(1)))
        .map(|w| w.iter().sum::<f64>() / w.len() as f64)
        .collect();
    let np = (roll.iter().map(|x| x.powi(4)).sum::<f64>() / roll.len().max(1) as f64).powf(0.25);
    let dur = pw_vals.len() as f64;
    let iff = np / FTP;
    let tss = dur * np * iff / (FTP * 3600.0) * 100.0;
    let summary = json!({
        "duration_s": dur, "np_w": (np * 10.0).round() / 10.0, "if": (iff * 1000.0).round() / 1000.0,
        "tss": (tss * 10.0).round() / 10.0, "compliance": "synthetic", "laps": []
    });
    Ok(Ride { frames, samples, steps, logs, compiled: serde_json::to_value(&compiled)?, summary })
}

fn build(fecap: &Path, out: &Path, stage: &Path, mode: RawMode, redacted: bool) -> R<()> {
    let text = fs::read_to_string(fecap)?;
    let lines = parse(&text);
    let r = ride(&lines, mode, redacted)?;
    let mut ws = WriteSession::create(stage, "crank.ride", "ride-0001", "one crank ride (spike #490)", TS)?;
    ws.with_extra(
        "crank.manifest",
        json!({
            "ride_id": "ride-0001", "start": TS, "end": "2026-11-03T00:03:17Z",
            "app_version": "0.0.0-spike", "app_commit": "c3b050b", "ftp_w": FTP,
            "devices": [{"role": "trainer", "id": "trainer-1"}], "redacted": redacted,
            "frames_format": ".fecap v1", "frames_verbatim_lines": r.frames.verbatim,
        }),
    )?;
    ws.with_extra("crank.summary", r.summary.clone())?;
    let (b, p) = blob_block("workout/dsl", "workout.dsl", Some("text/plain"), DSL.as_bytes().to_vec())?;
    ws.append_block(b, &p.bytes)?;
    let (b, p) = blob_block("workout/compiled", "compiled.json", Some("application/json"), serde_json::to_vec(&r.compiled)?)?;
    ws.append_block(b, &p.bytes)?;
    for (name, (spec, data)) in [("workout/steps", &r.steps), ("samples", &r.samples), ("logs", &r.logs)] {
        let (b, p) = table::table_block(name, spec, data)?;
        ws.append_block(b, &p.bytes)?;
    }
    let (b, p) = table::table_block("frames", &r.frames.spec, &r.frames.data)?;
    ws.append_block(b, &p.bytes)?;
    if let Some(raw) = r.frames.blob {
        let (b, p) = blob_block("frames_raw", "frames.bin", Some("application/octet-stream"), raw)?;
        ws.append_block(b, &p.bytes)?;
    }
    ws.seal(out)?;
    Ok(())
}

/// Per-block sizes of a sealed product (zip entry sizes are STORED, so payload == on-disk).
fn sizes(tsra: &Path) -> R<()> {
    let mut rd = Reader::open(tsra)?;
    let names: Vec<String> = rd.manifest().blocks.iter().map(|b| b.name.to_string()).collect();
    let mut sum = 0;
    for n in &names {
        let len = rd.read_block(n)?.len();
        sum += len;
        println!("{n:<18} {len:>8} B");
    }
    let total = fs::metadata(tsra)?.len();
    println!("{:<18} {sum:>8} B\n{:<18} {total:>8} B (whole .tsra: blocks + manifest + zip)", "sum(blocks)", "file");
    Ok(())
}

fn to_fecap(tsra: &Path, out: &Path) -> R<()> {
    let mut rd = Reader::open(tsra)?;
    let m = rd.manifest().clone();
    let fref = m.blocks.iter().find(|b| b.name.to_string() == "frames").ok_or("no frames")?;
    let spec: TableSpec = serde_json::from_value(fref.spec.clone())?;
    let data = table::decode(&spec, &rd.read_block("frames")?)?;
    let get = |n: &str| data.iter().find(|(k, _)| k == n).map(|(_, c)| c);
    let ColumnData::U64(t) = get("t_us").ok_or("t_us")? else { return Err("t_us dtype".into()) };
    let ColumnData::U8(dir) = get("dir").ok_or("dir")? else { return Err("dir dtype".into()) };
    let ColumnData::U8(len) = get("len").ok_or("len")? else { return Err("len dtype".into()) };
    let raw_blob = if m.blocks.iter().any(|b| b.name.to_string() == "frames_raw") {
        Some(rd.read_block("frames_raw")?)
    } else {
        None
    };
    let extra = serde_json::to_value(&m)?;
    let verb = extra.pointer("/extra/crank.manifest/frames_verbatim_lines").cloned().unwrap_or(json!([]));
    let verb: Vec<(u64, String)> = verb
        .as_array()
        .ok_or("verbatim")?
        .iter()
        .map(|v| (v["before_row"].as_u64().unwrap_or(0), v["text"].as_str().unwrap_or("").to_string()))
        .collect();
    let mut out_s = String::new();
    let mut vi = 0;
    let mut off = 0usize;
    for i in 0..t.len() {
        while vi < verb.len() && verb[vi].0 == i as u64 {
            out_s.push_str(&verb[vi].1);
            vi += 1;
        }
        let l = usize::from(len[i]);
        let bytes: Vec<u8> = if let Some(b) = &raw_blob {
            b[off..off + l].to_vec()
        } else if let Some(ColumnData::Utf8(h)) = get("raw_hex") {
            h[i].split(' ').map(|x| u8::from_str_radix(x, 16)).collect::<Result<_, _>>()?
        } else {
            (0..13)
                .map(|k| match get(&format!("b{k:02}")) {
                    Some(ColumnData::U8(c)) => Ok(c[i]),
                    _ => Err("no raw payload in this product"),
                })
                .collect::<Result<_, _>>()?
        };
        off += l;
        out_s.push_str(&render(t[i], dir[i] == 0, &bytes));
        out_s.push('\n');
    }
    for v in &verb[vi..] {
        out_s.push_str(&v.1);
    }
    fs::write(out, out_s)?;
    Ok(())
}

/// Write one block's raw Vortex bytes (for DuckDB's vortex extension) or CSV.
fn export(tsra: &Path, block: &str, out: &Path) -> R<()> {
    let mut rd = Reader::open(tsra)?;
    let bytes = rd.read_block(block)?;
    fs::write(out, bytes)?;
    Ok(())
}

/// Crash probe: append one small table block every 50 ms until killed.
fn crash_child(stage: &Path) -> R<()> {
    let mut ws = WriteSession::create(stage, "crank.ride", "crash", "crash probe", TS)?;
    ws.with_extra("crank.manifest", json!({"ride_id": "crash"}))?;
    let spec = TableSpec { columns: vec![col("t_s", "u4"), col("power_w", "u2")], rows: 60, row_index: None, partition: None };
    for i in 0u32.. {
        let data: TableData = vec![
            ("t_s".into(), ColumnData::U32((i * 60..i * 60 + 60).collect())),
            ("power_w".into(), ColumnData::U16(vec![200; 60])),
        ];
        let (b, p) = table::table_block(&format!("samples_chunk/c{i:05}"), &spec, &data)?;
        ws.append_block(b, &p.bytes)?;
        println!("committed {i}");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(())
}

fn recover(stage: &Path, out: &Path) -> R<()> {
    let ws = WriteSession::recover(stage)?;
    println!("recovered watermark {} blocks", ws.watermark());
    let m = ws.seal(out)?;
    println!("sealed {} blocks", m.blocks.len());
    let mut rd = Reader::open(out)?;
    let names: Vec<String> = rd.manifest().blocks.iter().map(|b| b.name.to_string()).collect();
    for n in &names {
        rd.read_block(n)?; // digest-verified read
    }
    println!("all {} blocks read back and verified", names.len());
    Ok(())
}

fn mode_of(s: &str) -> R<RawMode> {
    Ok(match s {
        "lenblob" => RawMode::LenBlob,
        "hex" => RawMode::Hex,
        "cols13" => RawMode::Cols13,
        "none" => RawMode::None,
        _ => return Err(format!("mode {s}").into()),
    })
}

fn main() -> R<()> {
    let a: Vec<String> = std::env::args().collect();
    let p = |i: usize| Path::new(&a[i]);
    match a.get(1).map(String::as_str) {
        // build <fecap> <out.tsra> <stage> <mode> <redacted 0|1>
        Some("build") => build(p(2), p(3), p(4), mode_of(&a[5])?, a[6] == "1"),
        Some("sizes") => sizes(p(2)),
        Some("fecap") => to_fecap(p(2), p(3)),
        Some("export") => export(p(2), &a[3], p(4)),
        Some("crash-child") => crash_child(p(2)),
        Some("recover") => recover(p(2), p(3)),
        _ => Err("usage: build|sizes|fecap|export|crash-child|recover".into()),
    }
}
```

`crash.sh` (the kill -9 probe):

```sh
#!/bin/bash
# Kill -9 a writer mid-ride, then recover + seal + verify.
B=$CARGO_TARGET_DIR/release/ride490
W=/private/tmp/claude-502/spike-490/work
S=$W/crash-stage-$1
$B crash-child $S > $W/crash-child-$1.log 2>&1 &
pid=$!
perl -e 'select(undef,undef,undef,1.3)'
kill -9 $pid
wait $pid; echo "child exit=$? (137 = SIGKILL)"
echo "last line printed by child: $(tail -1 $W/crash-child-$1.log)"
echo "journal lines: $(wc -l < $S/journal* 2>/dev/null || ls $S)"
ls $S
$B recover $S $W/crash-$1.tsra; echo "recover exit=$?"
```

## Appendix B: footprint crate

```toml
[package]
name = "footprint490"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
tessera-core = { git = "https://github.com/vig-os/tessera", rev = "a3df683b2574f5257b99c09710b49629c9daa17d" }
tessera-io = { git = "https://github.com/vig-os/tessera", rev = "a3df683b2574f5257b99c09710b49629c9daa17d" }
serde_json = "1"

[workspace]
```

```rust
//! Footprint: what crank would call — write a table + a blob through a WriteSession, seal, read back.
use tessera_core::block::table::{Column, TableSpec};
use tessera_io::table::{self, ColumnData, TableData};
use tessera_io::{Reader, WriteSession, blob_block};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let dir = std::path::Path::new(&a[1]);
    let mut ws = WriteSession::create(&dir.join("stage"), "crank.ride", "r", "d", "2026-11-03T00:00:00Z")?;
    ws.with_extra("crank.summary", serde_json::json!({"np_w": 200}))?;
    let spec = TableSpec { columns: vec![Column::new("t_s", "u4"), Column::new("power_w", "u2")], rows: 3, row_index: None, partition: None };
    let data: TableData = vec![("t_s".into(), ColumnData::U32(vec![0, 1, 2])), ("power_w".into(), ColumnData::U16(vec![200, 201, 202]))];
    let (b, p) = table::table_block("samples", &spec, &data)?;
    ws.append_block(b, &p.bytes)?;
    let (b, p) = blob_block("workout/dsl", "w.dsl", Some("text/plain"), b"1m 50%".to_vec())?;
    ws.append_block(b, &p.bytes)?;
    let out = dir.join("r.tsra");
    ws.seal(&out)?;
    let mut rd = Reader::open(&out)?;
    let back = table::decode(&spec, &rd.read_block("samples")?)?;
    println!("{}", back.len());
    Ok(())
}
```
