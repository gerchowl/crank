# Workout text DSL

`crank_domain::workout::parse_dsl(text, ftp, &mut ids)` returns a `Result<DslDoc, DslError>`: the text's
variable declarations (`DslDoc.vars`) and its tree (`DslDoc.nodes`); `print_dsl(vars, nodes)` turns them back
into canonical text. The two round-trip: `parse_dsl(print_dsl(v, t), ftp, &mut ids)` equals `(v, t)` structurally (ids
aside), and `print_dsl` of that is byte-identical. Loosely inspired by intervals.icu.

Variables (`$thr = 95%`, [ADR-0008](adr/0008-dsl-auto-ramps-variables.md) §2.6–§2.11, #742) are implemented; see
[Variables](#variables). Accepted, not implemented: auto ramps pinned to the neighbouring block (`ramp auto-75%`,
ADR-0008 §2.1–§2.5, #743); until it lands, `auto` is an unknown word as before.
Accepted, not implemented: ranged repeats that end on a ride-time HR condition (`3..10x[hr>=170](…)`) and no
script layer, [ADR-0010](adr/0010-workout-language-ranged-repeats.md) (#736, accepted 2026-11-27).

```text
"Warm-up" 8m ramp 45-75% 90rpm
3m 60%
"VO2 sets" 3x[+5w](5x(30s 125%, 15s 50%), "Recover" hr<118 2m..4m 40%)
"Threshold" 2x[!last](4m 95% 90-95rpm, 2m 55%)
"Cool-down" 5m ramp 60-40%
```

## Grammar

```ebnf
workout   = sep* { decl sep+ } [ item { sep+ item } ] sep* ;   (* decl: see Variables *)
sep       = "," | ";" | newline ;              (* all three are equivalent *)
item      = [ label ] ( repeat | step ) ;
label     = '"' { char | '\' any } '"' ;      (* '\' any = that character, a newline included; a raw newline is unterminated *)

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

## Variables

Declarations come before the first block, one per line (or separated as items are), and a declaration may use only
the variables declared above it: there are no forward references, so no cycles. One scope, the whole workout; an
unused variable is kept. Arithmetic is allowed in declarations only; a block uses a variable by name.

```text
$thr = 95%
$on = 4m
$rest = $on / 2
$sets = 3
$tempo = $thr - 10%

"Warm-up" 10m ramp 45%-$thr
"Threshold" $sets x[!last]("On" $on $thr 90rpm, "Off" $rest 55%)
"Cool-down" 5m ramp $tempo-40%
```

```ebnf
decl      = VAR "=" expr ;                           (* spaces around "=": $thr=95% is refused *)
VAR       = "$" letter { letter | digit | "_" } ;    (* ASCII, case-sensitive, at most 32 characters after "$" *)
expr      = term { ( "+" | "-" ) term } ;            (* binary operators are standalone words *)
term      = factor { ( "*" | "/" ) factor } ;
factor    = "-" factor | literal | VAR | "(" expr ")" ;  (* "-" attached (-5w, -$x) or a standalone prefix word *)
literal   = NUMBER "%" | NUMBER "w" | DURATION | NUMBER ;  (* % FTP, watts (decimals allowed), seconds, a number *)

(* where a block uses a variable; each is one word unless a space is shown *)
end       = … | VAR ;                                (* a duration *)
bound     = DURATION | VAR ;                         (* hr<118 $min..$max, hr<118 $z..4m *)
target    = … | VAR                                  (* % FTP gives a % target, watts a w target *)
          | pend "-" pend                            (* a range with at least one VAR: $lo-$hi, $lo-80%, 60%-$hi *)
          | "ramp" pend "-" pend ;                   (* ramp $a-$b, ramp $a-75%, ramp 45%-$b *)
pend      = NUMBER "%" | VAR ;                       (* % FTP only; every number carries its own % *)
repeat    = … | VAR " " "x" [ options ] "(" workout ")" ;   (* $sets x(…): two words *)
option    = … | "+" VAR ;                            (* [+$p]: watts or % FTP *)
```

**Types** are the unit of the value, with no annotations: % FTP (`95%`), watts (`250w`), duration (`4m`, `1m30s`)
and a plain number (`3`, `0.5`). `+` and `-` take two values of one type, `*` a value and a plain number (either
order), `/` a value divided by a plain number. % FTP and watts never combine, so `$thr - 10%` is percentage points
(95 % → 85 %); the relative form is `$thr * 0.9`. Cadence, heart rate and kJ take no variables.

**Exact arithmetic.** Declarations are evaluated as exact rationals (`num_rational::Ratio<i128>`, checked), so
`$d = 1m30s * 1.1` is exactly 99 s. A literal or a result outside that range is `NumericOverflow`, a zero divisor
`DivisionByZero`.

**Rounding, once, at each use**, to the printer's precision for that position (ties towards +∞): a `w` target to
whole watts (`5m $a` with `$a = 250w / 3` is 83 W), `%` targets, range and ramp ends and progressions to 0.001
(`100% / 3` is 33.333 %). No intermediate value is rounded: `$a = 250w / 3`, `$b = $a * 3` gives exactly 250 W.
Durations and repeat counts are never rounded: a duration must be whole seconds, 0 or more (a 0 s time step is the
compiler's `InvalidDuration` issue, `hr<118 0s..4m` is valid), a count a whole number ≥ 1. A power must be 0 or
more, a range ordered and at most 400 % FTP, as for a literal; a progression may be negative (`$drop = -5w`, then
`[+$drop]`).

**The tree keeps names.** A field a block can bind is a `Num`: a number, or `{ "var": "thr" }` (serde untagged;
TypeScript `number | { var: string }`), in `StepEnd::Time.s`, `HrRecovery.{min_s, max_s}`, every `PowerTarget` power
field, `RepeatNode.count` and `Progression.{watts_per_rep, pct_per_rep}`. `Workout.vars` holds the declarations
(`VarDecl { name, expr }`, `Expr` = `lit` (its exact decimal text and unit) | `var` | `neg` | `bin`); it defaults to
empty, so every stored workout reads unchanged. `resolve_vars(&Workout)` evaluates the declarations and returns the
tree with only literals; `compile_workout` runs it first when the workout declares or uses a variable, and its
`ResolveError` (an unknown name, a mismatched type, a value its field cannot hold, for a tree built or deserialised
directly) is `CompileError::Resolve`. The ride snapshot keeps the workout as written.

**Bounds** (stack safety, like `MAX_NESTING_DEPTH`): at most 100 declarations (`MAX_VARIABLES`,
`TooManyVariables { max }`), an expression at most 32 levels deep (`MAX_EXPRESSION_DEPTH`, `ExpressionTooDeep { max }`;
every operator, unary minus and parenthesis is a level, so `1w + 1w + …` of 33 operators is refused too), names of at
most 32 characters (`MAX_VARIABLE_NAME`, `VariableNameTooLong { max }`). Each is accepted at its limit and refused one
past it.

**Backward compatibility.** The lexer is unchanged, so `$thr`, `=` and the operators are ordinary words; the parser
gives a new meaning only to words holding `$` and to the standalone words `=`, `*`, `/`, `+`, `-` inside a
declaration, none of which a valid text held before. Every text that parsed before parses to the same tree and
prints byte-identically.

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
default `!lower` flag is omitted, labels are re-quoted with `\`, `"` and a newline escaped by `\`, so
every label prints to text that reads back as the same label. Comments and blank lines are not kept.
Declarations print first, one per line in declaration order (`$name = expr`, one space around `=` and every binary
operator, parentheses only where the tree needs them: `$a - ($b - $c)` keeps them, `($a * 2) + $b` prints
`$a * 2 + $b`; literals in their canonical forms, `90s` → `1m30s`, `%` to three decimals, unary minus attached,
`-(…)` before parentheses), then a blank line, then the blocks. A use prints its `$name`, never its value; a
workout without declarations prints exactly as before.

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

Variables (ADR-0008 §2.14; each with its typed kind, all tested with their UTF-16 spans in
`crates/crank-domain/tests/dsl_vars.rs`). A fault of one variable inside a word (`$lo-$nope`, `$min..$max`) selects
that variable; a fault of the form selects the word:

| text | kind | message | highlighted |
| --- | --- | --- | --- |
| `4m $thr` (not declared) | `UnknownVariable` | Unknown variable '$thr' — declare it before the first block, e.g. $thr = 95% | `$thr` |
| `$a = $b * 2`, then `$b = 1m` | `UnknownVariable` | Unknown variable '$b' — a declaration can use only variables declared above it | `$b` |
| `3m 60%`, then `$x = 1m` | `DeclarationAfterBlock` | Variables are declared before the first block | `$x` |
| `$thr = 95%` twice | `DuplicateVariable` | '$thr' is already declared | the second `$thr` |
| `$d = 4m` and `10m $d` | `VariableTypeMismatch` | '$d' is a duration (4m); a power target is expected here, e.g. 95% or 250w | `$d` |
| `$p = 250w` and `5m ramp $p-75%` | `VariableTypeMismatch` | '$p' is in watts; a ramp is in % FTP | `$p-75%` |
| `$p = 250w` and `5m $p-80%` | `VariableTypeMismatch` | '$p' is in watts; a range is in % FTP | `$p-80%` |
| `$a = 95% + 250w` | `MismatchedUnits` | Cannot add % FTP and watts | `+` |
| `$a = 4m * 2m` | `InvalidProduct` | A duration can be multiplied only by a plain number | `*` |
| `$a = 95% / 4m` | `InvalidQuotient` | A % FTP value can be divided only by a plain number | `/` |
| `$a = 1m / 0` | `DivisionByZero` | Division by zero | `/` |
| `$n = 7 / 2` and `$n x(…)` | `InvalidRepeatCount` | Repeat count '$n' is 3.5; it must be a whole number ≥ 1 | `$n` |
| `$n = 0` and `$n x(…)` | `InvalidRepeatCount` | Repeat count '$n' is 0; it must be a whole number ≥ 1 | `$n` |
| `$d = 4m / 7` and `$d 90%` | `FractionalDuration` | '$d' is 34.286 s; a duration must be whole seconds | `$d` |
| `$d = 1m - 2m` and `$d 50%` | `NegativeDuration` | '$d' is -60 s; a duration must be 0 or more | `$d` |
| `$e = 40% - 50%` and `5m $e` | `NegativePower` | '$e' is -10 % FTP; a power must be 0 or more | `$e` |
| `$lo = 80%`, `$hi = 60%` and `5m $lo-$hi` | `ReversedPowerRange` | Range low end is above high end (use 'ramp 80-60%' for a descending ramp) | `$lo-$hi` |
| `$a = $b-30s` | `UnspacedOperator` | Put spaces around '-' in an expression: $b - 30s | `$b-30s` |
| `$thr=95%` | `UnspacedEquals` | Put spaces around '=': $thr = 95% | `$thr=95%` |
| `$1x = 3` | `InvalidVariableName` | A variable name starts with a letter, e.g. $thr, $rest_2 | `$1x` |
| `ramp $a-75` / `$lo-80` | `MissingPercent` | Write the % on the number next to a variable, e.g. ramp $a-75% | `$a-75` / `$lo-80` |
| `ramp 45%-75%` | `RedundantPercent` | Write a ramp between two numbers as ramp 45-75% | `45%-75%` |
| `5m 60%-80%` | `RedundantPercent` | Write a range between two numbers as 60-80% | `60%-80%` |
| `$p = 5w` and `3x[-$p](…)` | `InvalidProgressionVariable` | Write a falling progression as a negative variable: $p = -5w, then [+$p] | `-$p` |
| `$a = 5w 3w` | `ExpectedOperator` | Expected an operator (+, -, *, /) or the end of the declaration; got '3w' | `3w` |
| `$a = 5q` | `ExpectedExpression` | Expected a value (95%, 250w, 4m, 3) or a variable; got '5q' | `5q` |
| `$a = (5w + 3w` | `UnclosedParenthesis` | Unclosed '(' — missing ')' | `(` |

## Ids

`parse_dsl` takes an explicit `&mut impl FnMut() -> String` ID source. The domain
has no hidden counter, clock or randomness. The caller owns uniqueness: reuse one
source across parses for fresh unique identities, or reset it for reproducible
identities. Successful parses consume one ID per node; a parse error may consume
IDs for earlier nodes, while a lexical error consumes none. For example:

```rust
let mut next = 0;
let mut ids = || { next += 1; format!("workout:{next}") };
let nodes = crank_domain::workout::parse_dsl("1m 50%", 280.0, &mut ids)?.nodes;
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
All numbers use `f64` to retain JavaScript numeric semantics (a bindable field is a `Num`, whose literal is
that `f64`). The frozen TS reference has no variables: parity covers today's grammar only, and evaluation exists only
in the Rust core. Parity tests load
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
when present, and absent or true `skipLastRest`. Labels survive re-quoting
whatever they hold. Generated property tests cover every target/end form,
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

## Grade profiles (routes)

A route's grade profile is a sibling grammar, not an extension of this one (ADR-0005 §2.2.5, §3): it shares
this lexer, labels, separators, comments, `N x(...)` repeats (without `[...]` options) and the error spans
(UTF-16), but a segment is `DIST grade [surface]`, e.g. `"Col" 5km 6.5%, 3km ramp 6.5%..9% gravel`.
Distances are in `km` only (0.001 km resolution); `500m` is refused because `m` is minutes here, and
workout durations, step ends and power targets are refused naming the workout form. A `$x` or `auto` word is refused with
the message and span it always had: the grade grammar gains no variables. Grades are
−25 % to +30 %, segments 0.01 to 100 km, 1 000 km and 32 nesting levels in all. The canonical printer
writes one top-level item per line, lower-case keywords, km with at most three decimals and grades
in their shortest exact form, so parse → print → parse is equal and printing is idempotent
(`crank_domain::route::{parse_grade_profile, print_grade_profile, parse_route_dsl, print_route_dsl}`,
property-tested in `crates/crank-domain/tests/route_dsl.rs` and, near the size limits, `route_r3.rs`).

Input limits (ADR-0005 §2.2.4, the same numbers): the text is at most 256 KiB (UTF-8 bytes,
`MAX_PROFILE_TEXT_BYTES`), a label at most 100 characters (`MAX_LABEL_CHARS`), and the labels of the
expanded route (each repeated segment carrying its own or its repeat's label) at most 1 MiB
(`MAX_EXPANDED_LABEL_BYTES`), computed from the repeat counts before anything is expanded. The
canonical form is bounded too: a profile whose canonical text would exceed 256 KiB is refused when it
is parsed (`CanonicalTooLong`), even if the text as given fits, so every profile the parser accepts
prints to text it accepts again.
