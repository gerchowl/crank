# Workout text DSL

`crank_domain::workout::parse_dsl(text, ftp)` returns a `Result<Vec<WorkoutNode>, DslError>`;
`print_dsl(nodes)` turns a tree back
into canonical text. The two round-trip: `parse_dsl(print_dsl(t), ftp)` equals `t` structurally (ids
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

On failure `parse_dsl` returns `Err(DslError { message, offset, length })` where `offset/length`
select the offending token in the original text (length 0 = end of input; an unclosed `(` points
at the opening parenthesis). Examples:

| text | message | highlighted |
| --- | --- | --- |
| `3x(1m 120%, 1m 50%` | Unclosed '(' — missing ')' | `(` |
| `5q 120%` | Expected a duration (30s, 4m, 1m30s), kJ (10kj), 'lap', 'hr<N' or a repeat (3x(...)); got '5q' | `5q` |
| `10m` | Missing power target (e.g. 120%, 250w, 50-75%, ramp 50-75%, 140bpm, free) | `10m` |
| `hr<120 4m 40%` | Expected min..max duration after 'hr<120', e.g. 2m..4m | `4m` |
| `5m 900%` | 900 % FTP is implausible (max 400 %) | `900%` |
| `10 50%` | '10' needs a unit: 30s, 4m, 1h, 10kj — or '10x(...)' for a repeat | `10` |

## Ids

Every `parse_dsl` call assigns **fresh** unique ids. When syncing text → tree in an editor, carry
selection by `FlatBlock.path`/tree path, not by id. When inserting snippets or duplicating subtrees, assign fresh identities to keep ids unique.

## Rust port notes (ADR-0001 §3–4, #68)

The frozen reference remains in `shared/`; this document is the production grammar.
The domain contract derives serde and, with feature `specta`, `specta::Type`.
Optional fields serialize as `null` as §4 requires; the frozen mock omits them.
Parity tests normalize only those schema-defined optional fields on the reference.
All numbers use `f64` to retain JavaScript numeric semantics; grammar-constrained
integers compare exactly, and decimal-capable fields use §3's relative `1e-12` /
absolute `1e-15` tolerance, including integer-valued decimals (#74).
Error spans count UTF-16 code units, not UTF-8 bytes or Unicode scalar values.
IDs use a process-local atomic sequence, without I/O, clocks or randomness.

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
