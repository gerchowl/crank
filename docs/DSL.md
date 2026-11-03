# Workout text DSL

`crank_domain::workout::parse_dsl(text, ftp, &mut ids)` returns a `Result<Vec<WorkoutNode>, DslError>`;
`print_dsl(nodes)` turns a tree back
into canonical text. The two round-trip: `parse_dsl(print_dsl(t), ftp, &mut ids)` equals `t` structurally (ids
aside), and `print_dsl` of that is byte-identical. Loosely inspired by intervals.icu.

```text
"Warm-up" 8m ramp 45-75% 90rpm
3m 60%
"VO2 sets" 3x[+5w](5x(30s 125%, 15s 50%), "Recover" hr<118 2m..4m 40%)
"Threshold" 2x[!last](4m 95% 90-95rpm, 2m 55%)
"Cool-down" 5m ramp 60-40%
```

## Grammar

```ebnf
workout   = sep* [ item { sep+ item } ] sep* ;
sep       = "," | ";" | newline ;              (* all three are equivalent *)
item      = [ label ] ( repeat | step ) ;
label     = '"' { char | '\"' | '\\' } '"' ;  (* no newlines inside *)

repeat    = COUNT "x" [ "[" option { [","] option } "]" ] "(" workout ")" ;
option    = SIGNED "w"                         (* progression.wattsPerRep, e.g. +5w, -10w *)
          | SIGNED "%"                         (* progression.pctPerRep (% FTP), e.g. +2% *)
          | "!last" ;                          (* skipLastRest *)

step      = end attr* ;                        (* exactly one power target is required *)
end       = DURATION                           (* { kind: "time" } *)
          | NUMBER "kj"                        (* { kind: "kj" } *)
          | "lap"                              (* { kind: "lap" } open-ended, ends on skip *)
          | "hr<" INT DURATION ".." DURATION ; (* { kind: "hr_recovery", targetBpm, minS, maxS } *)
attr      = target | cadence | flag ;
target    = NUMBER "%"                         (* pct_ftp *)
          | NUMBER "-" NUMBER "%"              (* range (lo ≤ hi), % FTP; ERG uses the midpoint *)
          | "ramp" NUMBER "-" NUMBER "%"       (* ramp from → to, may descend *)
          | INT "w"                            (* watts *)
          | INT "bpm"                          (* hr target: power is steered to hold this HR *)
          | "free" ;
cadence   = INT [ "-" INT ] "rpm" ;            (* 90rpm ⇒ {lo: 90, hi: 90} *)
flag      = "!lower" | "!cap" | "!coach" ;     (* hr_recovery onUnreachable; default !lower *)

DURATION  = [ INT "h" ] [ INT "m" ] [ INT "s" ] ;   (* at least one part: 30s 4m 1h 1m30s 1h5m *)
COUNT     = INT ;  SIGNED = ("+" | "-") NUMBER ;  NUMBER = INT [ "." INT ] ;
comment   = "#" … end of line ;
```

Tokens are separated by whitespace or by `( ) [ ] , ; "`. Keywords and units are
case-insensitive (`4M`, `RAMP`, `LAP` are accepted; printing emits lower case).

## Compilation semantics (owned by #69)

- **Nested repeats** flatten depth-first; `FlatBlock.path` holds child indices into the tree and
  `iteration` the rep counter of every enclosing repeat.
- **`!last` (skipLastRest)**: on the final iteration the repeat's *last child* is skipped (only if
  the repeat has more than one child) — typically the rest after the last interval.
- **Progression** `[+5w]` / `[+2%]`: rep *i* (0-based) adds *i*·value to every **work** step
  (≥ 80 % FTP) inside, cumulatively through nested repeats. Rests are not progressed.
- **`hr<118 2m..4m 40%`**: ride at 40 % until HR < 118 bpm, but never shorter than 2 min nor
  longer than 4 min. If the fitted HR asymptote stays above the target, `onUnreachable` decides:
  `!lower` (default) drops the recovery power by 25 %, `!cap` just runs to the max,
  `!coach` asks the coach (a pending `replan` nudge; accept = end the block now).
  The compiled duration is an estimate (usual τ), `durRangeS = [min, max]`.
- **`lap`**: expected 5 min, range 30 s – 30 min; the ride moves on when the rider skips (`S`).
- **`10kj`**: duration estimated from the target power; in the ride it ends when the block's
  work reaches the target.
- **Phases**: work = ≥ 80 % FTP; easy blocks before the first work block are `warmup`, after the
  last one `cooldown`, in between `recovery`; `hr<` steps are `recovery`, `free` is `free`.
  Labels containing "warm"/"cool" force warm-up/cool-down.

## Canonical printing

`print_dsl` emits one top-level item per line and comma-separated items inside repeats; durations
are normalised (`90s` → `1m30s`, `3600s` → `1h`), options print as `[+5w, +2%, !last]`, the
default `!lower` flag is omitted, labels are re-quoted. Comments and blank lines are not kept.

## Errors

On failure `parse_dsl` returns `Err(DslError { kind, message, offset, length })` where `offset/length`
select the offending token in the original text (length 0 = end of input; an unclosed `(` points
at the opening parenthesis). Match the typed `DslErrorKind` to handle failures;
`message` is display text retained for exact TS parity, not a discriminator.
Kinds distinguish all reachable error classes; invalid repeat counts, reversed
recovery bounds and implausible intensities also carry numeric details. Examples:

| text | message | highlighted |
| --- | --- | --- |
| `3x(1m 120%, 1m 50%` | Unclosed '(' — missing ')' | `(` |
| `5q 120%` | Expected a duration (30s, 4m, 1m30s), kJ (10kj), 'lap', 'hr<N' or a repeat (3x(...)); got '5q' | `5q` |
| `10m` | Missing power target (e.g. 120%, 250w, 50-75%, ramp 50-75%, 140bpm, free) | `10m` |
| `hr<120 4m 40%` | Expected min..max duration after 'hr<120', e.g. 2m..4m | `4m` |
| `5m 900%` | 900 % FTP is implausible (max 400 %) | `900%` |
| `10 50%` | '10' needs a unit: 30s, 4m, 1h, 10kj — or '10x(...)' for a repeat | `10` |

## Ids

`parse_dsl` takes an explicit `&mut impl FnMut() -> String` ID source. The domain
has no hidden counter, clock or randomness. The caller owns uniqueness: reuse one
source across parses for fresh unique identities, or reset it for reproducible
identities. Successful parses consume one ID per node; a parse error may consume
IDs for earlier nodes, while a lexical error consumes none. For example:

```rust
let mut next = 0;
let mut ids = || { next += 1; format!("workout:{next}") };
let nodes = crank_domain::workout::parse_dsl("1m 50%", 280.0, &mut ids)?;
```

## Tree-helper ownership

When syncing text → tree in an editor, carry selection by tree path, not by ID.
`re_id` and `find_node` (#152) are ported from the reference `reId` / `findNode`
and live in `workout/tree.rs`. They are editor/tree utilities, not compiler
dependencies: the reference compiler imports neither.

- `re_id(nodes, &mut ids) -> Result<Vec<WorkoutNode>, TreeError>` deep-copies a
  subtree (snippet insertion, duplication) and replaces the ID of every step and
  repeat with one drawn from the caller-owned `ids` source, requested in
  pre-order (a repeat before its children, siblings in order). Everything but IDs
  is preserved exactly. The helper does not check uniqueness: supply a source that
  is fresh across the whole workout (the reference's `uid` is a global counter;
  the Rust port has none). Repeats nested deeper than `MAX_NESTING_DEPTH` return
  `TreeError::NestingTooDeep` before any ID is consumed, as the compiler does.
- `find_node(nodes, id) -> Option<&WorkoutNode>` is a depth-first pre-order
  lookup. With duplicate IDs the first match in that order wins (a repeat is
  tested before its children), as in the reference; a missing ID gives `None`.
  It is iterative and safe on trees of any depth.

`withoutIds` is a private Rust test helper; it is not a production API.
Reference parity is tested against `tests/fixtures/tree_helpers.json`, generated
from the frozen TS by `tests/fixtures/export-tree-reference.test.ts` (random TS
IDs are reduced to their `s`/`r` kind prefix).

## Rust port notes (ADR-0001 §3–4, #68)

The frozen reference remains in `shared/`; this document is the production grammar.
The domain contract derives serde and, with feature `specta`, `specta::Type`.
Optional fields serialize as `null` as §4 requires; the frozen mock omits them.
Parity tests normalize only those schema-defined optional fields on the reference.
All numbers use `f64` to retain JavaScript numeric semantics. Parity tests load
`tests/vectors/schema.json` from #74 directly: repeat counts and UTF-16 spans
compare as exact integral integers; physical quantities (including whole-valued
durations, watts, HR and cadence) use §3's relative `1e-12` / absolute `1e-15`
tolerance. Both compared documents must have full schema coverage before any
mismatch is returned; unknown fields and numeric classifications fail loudly.
Error spans count UTF-16 code units, not UTF-8 bytes or Unicode scalar values.

The unreachable non-EOF check after `parseSeq(null)` in `dsl.ts:121` and the
unlabelled EOF alternative in `dsl.ts:152` are deliberately omitted: sequence
parsing consumes all top-level items and handles bare EOF before item parsing.
The dangling-label EOF error remains reachable and covered.

The printer rounds durations, watts and HR to integers, and percentages, kJ and
progression to three decimal places, using JavaScript's ties-towards-positive-
infinity rule. Structural round trips apply to DSL-representable trees: positive
integer step durations, integer HR/watts/cadence/count, decimal precision within
three places, ordered ranges, nonempty repeats, at least one progression field
when present, and absent or true `skipLastRest`. Labels must survive re-quoting
without raw newlines. Generated property tests cover every target/end form,
labels, cadence, signed progression, and nested repeats under these constraints.

Canonical number-format parity is bounded to finite numeric magnitudes whose
printed values remain below `1e21` in absolute value. Decimal fields round to
three places, so their smallest nonzero printed magnitude is `0.001`; the JS
exponent threshold below `1e-6` cannot occur. JS switches to exponent notation at
`1e21`, while Rust Display remains decimal. The parser accepts finite numbers
outside this bound; printing/round-trip parity is not promised there. Integers above `2^53 - 1` also inherit JS's loss of integer
precision. Whole-number fields use shortest decimal formatting after rounding,
rather than fixed precision, and both signs of zero print as `0`.

## Rust input safety bounds

The Rust parser accepts at most `MAX_NESTING_DEPTH` (32) enclosing repeats;
32 repeats around a step succeed, while a 33rd returns
`DslErrorKind::NestingTooDeep { max: 32 }` at that repeat's count token.
This is a Rust-side stack-safety bound absent from the frozen TS reference.
Both the maximum-depth success and a 2,000-level rejection are tested on a
256 KB worker stack in the default nextest gate. That guarantee is measured on
debug builds (the worst case for frame size); a 64 KB stack overflows.
Sibling items do not consume nesting depth. The bound also limits the depth of
trees returned by the parser; callers constructing trees directly must manage
their own traversal depth (`re_id` rejects trees past the bound; `find_node` is
iterative).

Numeric literals that overflow `f64` and duration arithmetic that produces
infinity return `DslErrorKind::NumericOverflow` instead of an infinite AST
field. This Rust-side restriction also differs from the TS reference.
Overflowing literals are rejected during lexing without consuming IDs;
duration arithmetic errors can consume IDs for preceding nodes.
Default-gate properties exercise arbitrary Unicode strings and the DSL alphabet,
including both closed and unclosed deep repeats, asserting completion with
success or a typed error. Each property runs 256 cases; DSL-alphabet bodies
are bounded to 2,048 characters and generated nesting to 2,048 levels.
