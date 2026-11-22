# ADR-0008: Workout DSL: auto ramps pinned to neighbouring blocks, and variables

- **Status:** Proposed (awaiting owner review; no code until accepted)
- **Date:** 2026-11-22
- **Issue:** #694
- **Related:** `docs/DSL.md` (the production grammar, #68), the compiler (#69, `crates/crank-domain/src/workout/compile.rs`),
  DESIGN §5.7 (editor, undo groups), §7.1 (coach bounds), §17 (live workout edits; §17.1 amended by §2.13.1), #111 (editor), #653 (outline edits),
  #110 (stored library), ADR-0001 §3–4 (domain contract, bindings)

## For the owner

This proposal amends the workout DSL. Nothing is implemented. Each line below is a decision that needs a yes or a
no; the section in brackets has the reasons and the rejected alternatives.

1. **Pinned ramp syntax** `ramp auto-75%`, `ramp 45%-auto`, `ramp auto-auto`: `auto` takes the place of a number,
   and in a form with `auto` the `%` sits on the number (§2.1).
2. **A pinned end takes the power of the block next to it on the ridden timeline**: the end of the block before,
   the start of the block after, with repeats, progression and `!last` applied, worked out for each occurrence
   (§2.2).
3. **No guessing**: pinning to nothing (first or last block), to a free ride, to a heart-rate target, or to an end
   that is itself `auto` (two ramps pinned to each other, or one ramp pinned to itself across a repeat) is an error
   that blocks Ride this, not a warning with a fallback (§2.3).
4. **A pinned ramp is never progressed** by `[+5w]` / `[+2%]`; its pinned end already follows the progressed
   neighbour (§2.4).
5. **Variables** are `$name = value` lines before the first block, one scope for the whole workout (§2.6).
6. **Arithmetic only in declarations** (`$rest = $work / 2`, `$tempo = $thr - 10%`); a block uses a variable by
   name only (`4m $thr`) (§2.8).
7. **The coach's ±% and the rider's ± apply after** variables and pins are resolved, as they apply today to the
   compiled blocks, and so do the rider's live edits (skip, extend, end set); a pin is not re-worked out during
   the ride (§2.13), except at the seams of a rep the rider adds (decision 11).
8. **An edit to one block never edits a variable**: changing a block's bound value from the outline or by an AI
   rewrite writes a number into that block (it unbinds); a variable is changed only through its own row or the
   text (§2.12).
9. **The tree keeps names**: a numeric field holds a number or `{ "var": "thr" }`, and the workout holds its
   declarations, rather than a second source tree beside the resolved one (§2.10).
10. **`$thr - 10%` is percentage points** (95 % → 85 %), not relative (95 % → 85.5 %); relative is written
    `$thr * 0.9`. Recommended because `-` then means the same for every unit (it subtracts two values of one type),
    and both meanings stay writable (§2.8).
11. **Amend DESIGN §17.1 for pins**: when the rider adds a rep (`repeat_once`), the pinned ends at the seams of the
    added rep are worked out again, so the result still equals compiling with one more rep; a seam ramp that has
    already started keeps its target (§2.13.1).

Everything else (error wording, canonical printing, limits, tests) follows from these.

## 1. Context

The owner asked (#694, 2026-11-22) for two things when writing a workout:

- **Auto ramps.** A ramp whose start and/or end power comes from the block before or after it: a warm-up ramp that
  ends at the first interval's power, a cool-down ramp that starts from the last block's power. Today the rider
  writes both ends (`ramp 45-95%`) and must keep the 95 in step with the interval by hand.
- **Variables.** A named power or duration used in several blocks, so changing it once changes all of them.

Facts this design rests on:

- **The grammar** is `docs/DSL.md`: `parse_dsl(text, ftp, ids)` returns the tree, `print_dsl(nodes)` its canonical
  text, and `parse(print(t)) == t` (ids aside) with printing idempotent, property-tested (#68). Ramps are % FTP only
  (`PowerTarget::Ramp { from, to }`); watts exist only as a flat target (`PowerTarget::Watts`).
- **The tree is the stored form, the text is derived.** `Workout { id, name, ftp, nodes }` is what the library holds
  and what a ride's `workout_json` snapshots; the editor shows `print_dsl` of the tree, and every outline action of
  #653 edits the tree and commits it with its canonical text (`editor/model.ts` `applied`). A name that lives only in
  the text would be lost by the first outline edit, so names must live in the tree.
- **The compiler flattens before it times.** `compile_workout` walks the tree depth-first into occurrences, each with
  its repeat iterations and progression offset (`Offset`, only for work steps, `is_work_step`: ≥ 80 % FTP,
  `WORK_THRESHOLD_PCT`), then works out watts (`target_watts`), durations, phases and validation. The ride engine
  consumes only the compiled `FlatBlock`s (`power_from`, `power_to`, in watts; `erg/policy.rs` interpolates between
  them). So anything that depends on the neighbour in time can be resolved inside the compiler and the engine does
  not change.
- **Ride-time changes are orthogonal.** The rider's intensity ± and the coach's ±% apply to the effective blocks'
  targets during the ride (DESIGN §7.1, §17); a rider edit never changes them.
- **Undo groups** (DESIGN §5.7): typing bursts end at a 500 ms pause (`GROUP_MS`), a structural edit is its own step.

### 1.1 How established formats do it

| Format | Ramps | Pinned to a neighbour? | Variables? |
|---|---|---|---|
| Zwift `.zwo` (XML) | `Warmup` / `Cooldown` / `Ramp` with `PowerLow`, `PowerHigh` as FTP fractions | No: both ends are written on every element | No |
| ERG / MRC (`[COURSE DATA]`, minutes and watts or % FTP per row) | The profile is a list of points joined by straight lines; a ramp is two rows with different values | Implicitly: a point is shared by the segment before and after it. The MRC_creator tool adds a row form that "creates a Ramp from previous value to this one" | No |
| intervals.icu workout text | `10m ramp 50%-75%`, `freeride` turns ERG off | No | Not observed in its syntax guide |
| TrainerRoad | Workout Creator is graphical; it imports ERG/MRC | (MRC rules) | No |

What we borrow: the `ramp a-b%` spelling is already intervals.icu's (DSL.md says "loosely inspired by
intervals.icu"); the **meaning** of a pinned end is MRC's shared point, "from the previous value" (MRC_creator), made
explicit per end instead of implied by every row; `freeride` having no planned power is why pinning to `free` is an
error (§2.3). None of these formats has variables, so §2.6–§2.9 borrow from programming practice instead: declare
before use (no forward references, hence no cycles) and a `$` sigil so a name can never collide with a keyword
(`lap`, `free`, `ramp`, `auto`).

## 2. Decision

### 2.1 Auto-ramp syntax

```ebnf
target    = … | "ramp" ramp_ends ;
ramp_ends = NUMBER "-" NUMBER "%"              (* today: ramp 45-75% *)
          | rend "-" rend ;                    (* new: at least one end is "auto" or a VAR (§2.9) *)
rend      = NUMBER "%" | VAR | "auto" ;        (* VAR: a % FTP variable *)
```

So `ramp 45%-auto` pins the end, `ramp auto-75%` the start, `ramp auto-auto` both (a bridge), and `ramp $wu-auto`,
`ramp $a-$b`, `ramp $a-75%` use variables. `auto` is a keyword (case-insensitive, printed lower case) valid only as a
ramp end. As today, the ends are one word token, split at its `-` (a name cannot contain `-`). **Where the unit
binds:** in today's form the one `%` at the end covers both numbers; in the new form every number carries its own
`%` and a variable carries its type, so `ramp 45-auto` and `ramp $a-75` are refused with a message showing the
fix, and `ramp 45%-75%` (no `auto`, no variable) is refused pointing at today's spelling `ramp 45-75%`, so each ramp
has exactly one spelling.

- **Chosen:** the number keeps its own `%` (`ramp 45%-auto`). With `auto` on one side the `%` cannot sit at the end of
  the word without hanging on a keyword, and every unit in the DSL is written on a number.
- **Rejected:** `ramp 45-auto%` (a unit on a keyword; reads as "auto percent"); a flag on an ordinary ramp
  (`ramp 45-95% !pin-end`) because it forces the writer to type a number that is then ignored, and the printed text
  would show a value that is not ridden.
- **Model:** a new variant, so every stored ramp keeps its JSON shape (§2.15):

  ```rust
  /// Linear ramp with at least one end pinned to the neighbouring block (ADR-0008 §2.2).
  PowerTarget::AutoRamp { from: Option<Num>, to: Option<Num> }   // None = pinned (`auto`), Num: % FTP
  ```

  The parser never builds `AutoRamp` with both ends present; a deserialised one with both present is refused by
  `compile_workout` with the `AutoRampNotPinned` error issue rather than treated as a plain ramp.

### 2.2 What a pinned end means

A pinned end takes, **for each compiled occurrence of the ramp**, the power of the block next to that occurrence on the
ridden timeline:

- pinned **start** = the previous block's `power_to` (its end power, so a ramp before it continues without a step);
- pinned **end** = the next block's `power_from`.

"Next to it on the timeline" is the compiler's flattened order, so the tree shape needs no special cases:

| Neighbour | Pinned start takes | Pinned end takes |
|---|---|---|
| a steady step (`%`, `w`, range, `kj`, `lap`) | its power (a range: the midpoint ERG rides) | its power |
| a ramp (explicit or auto) | its `power_to` | its `power_from` |
| a repeat | the last block **played** in its last iteration: progression included, and the second-last child when `!last` skips the last one | its first child's first block in iteration 0 |
| inside a repeat | iteration 0: the block before the repeat; later iterations: the last block of the previous iteration | the last iteration: the block after the repeat; earlier: the first block of the next iteration |
| `hr<118 2m..4m 40%` (HR recovery) | its planned power (40 %), not the `!lower` reduction the ride may apply | the same |

So `3x(1m ramp auto-120%, 2m 50%)` ramps from the block before the repeat in the first iteration and from 50 % in
the next two, and the profile shows exactly that.

Watts and % FTP mix freely: the compiled block is in watts anyway, so `ramp auto-90%` after `5m 200w` starts at
200 W. A ramp's own number stays % FTP.

- **Rejected:** resolving pins on the tree, before flattening (one value per ramp in the source). It cannot express
  the inside-a-repeat case (different neighbours per iteration), and it would pin the start after a repeat to a block
  that `!last` skips or progression has moved, so the rider would feel a step exactly where they asked for none.
- **Rejected:** "the repeat's boundary child" as the neighbour regardless of `!last` and progression: same step.

### 2.3 Neighbours that cannot be pinned to

Each is an **error** (Severity `Error`, so Ride this is disabled with the reason, DESIGN §5.7) carried as a
`ValidationIssue` with the ramp's `node_id`, so the editor marks the ramp:

| Case | `ValidationKind` | Message |
|---|---|---|
| pinned start and no block before it (the first occurrence of the first block) | `AutoRampNoNeighbour` | Ramp start 'auto' has no block before it — give the start a number, e.g. ramp 40%-auto |
| pinned end and no block after it | `AutoRampNoNeighbour` | Ramp end 'auto' has no block after it — give the end a number, e.g. ramp auto-40% |
| neighbour is `free` | `AutoRampUnplannedNeighbour` | Ramp 'auto' cannot follow or lead into a free ride block: it has no planned power |
| neighbour is a heart-rate target (`140bpm`) | `AutoRampUnplannedNeighbour` | Ramp 'auto' cannot follow or lead into a heart-rate block: its power is set during the ride |
| a pinned end whose neighbour's adjacent end is also `auto` (A's end pinned and B, right after A, with its start pinned), between two ramps | `AutoRampFacing` | This ramp's 'auto' end meets the 'auto' start of the next ramp ('<label or text of B>') — give one of these ends a number |
| the same, where A and B are one ramp in consecutive iterations (`3x(1m ramp auto-auto)`, or a repeat whose last child ends `auto` and whose first child starts `auto` with A = B) | `AutoRampFacing` | This ramp is pinned to itself across the repeat: its 'auto' end meets its own 'auto' start in the next iteration — give one of them a number |
| `AutoRamp` with no `auto` end (only by deserialising, §2.1) | `AutoRampNotPinned` | Ramp has no 'auto' end |

There is no SIM or other unpowered block in the workout grammar (routes are a sibling grammar, ADR-0005 §2.2.5); if
one is ever added it falls under `AutoRampUnplannedNeighbour` by the same reasoning as `free`.

**The rule, in one line:** a pinned end reads its neighbour's adjacent end, and that end must not be `auto`. A steady
block's end is its power; an explicit ramp's end is a number; an auto ramp's adjacent end must be a number. So there
are no chains to resolve: every pinned end is one look-up, and the only cycle a timeline can contain is two `auto`
ends facing each other, which the rule refuses. Examples:

- valid: `5m 50%, 30s ramp auto-auto, 10m 95%` (a bridge between two steady blocks);
- valid: `ramp 40%-80%, ramp auto-60%, ramp auto-90%` (each `auto` start meets a number end);
- refused: `ramp 40%-auto, ramp auto-auto, 10m 95%` (the first ramp's end and the second's start face each other);
- refused: `3x(1m ramp auto-auto)` (the ramp faces itself across the iteration boundary; iteration 0's start and
  iteration 2's end are fine on their own, but iterations 0→1 and 1→2 face).

Issues are reported once per node and kind, not once per occurrence (a repeat of 20 does not give 20 issues); a
facing pair is reported on the earlier ramp (the one whose end is pinned), naming the other.

An errored block still needs a value to draw: its unresolved end is drawn at its other end (a flat line), and an
`auto-auto` with neither end resolvable at 50 % FTP, the compiler's existing nominal for `free`. That is display only:
the error blocks riding, so the value is never sent to the trainer.

- **Rejected:** a warning with a fallback (start at 0 W, or at 40 % FTP). A workout would ride a ramp the writer did
  not write, silently in the ride; the quality bar forbids silent fallbacks.
- **Rejected:** a parse error. The parser does not know the timeline (repeats, `!last`, progression); the compiler
  does, and its issues already carry `node_id` for the editor.

### 2.4 Progression and phase of a pinned ramp

- A pinned end is the neighbour's compiled power, **progression included**. The ramp itself receives **no**
  progression offset, on either end: its pinned end has it already, and its number end stays as written. This makes
  `3x[+5w](ramp auto-auto 30s, 4m 100%, …)` behave as written: the bridge follows the intervals as they grow.
- Phase and work classification use the resolved ends: the occurrence's `max(power_from, power_to) / ftp` against
  80 %, as for explicit ramps (`target_pct` uses the maximum). A label containing "warm"/"cool" still forces the phase.
- **Rejected:** "an auto ramp compiles exactly like the explicit ramp with the pinned number written in", which would
  progress the pinned end a second time inside a progressing repeat.

### 2.5 How a pinned end is shown

- **Compiled blocks:** `FlatBlock` gains `pinned_from: bool` and `pinned_to: bool` (serde-skipped when false, so every
  existing compiled snapshot is unchanged). The engine ignores them.
- **Profile:** the resolved shape, drawn like any ramp; no special mark. The point of a pin is that the joint is
  continuous, which the profile shows by itself.
- **Outline:** the row reads its target as written: "Warm-up · 8:00 · ramp auto → 75 %". The value is not repeated in
  the row because it can differ per occurrence.
- **Summary block list:** watts as today, with "(auto)" after a pinned end: "Warm-up · 8:00 · 113 → 238 W (auto)".
- **Text:** as written; `print_dsl` prints `auto`.

### 2.6 Variables: declaration and scope

```ebnf
workout = sep* { decl sep+ } [ item { sep+ item } ] sep* ;
decl    = VAR "=" expr ;
VAR     = "$" letter { letter | digit | "_" } ;   (* ASCII, case-sensitive, at most 32 characters after "$" *)
```

- Declarations come **before the first block**; one after a block is an error. `print_dsl` prints them first, one per
  line, in declaration order.
- One scope: the workout. A declaration may use only variables declared **above** it, so there are no forward
  references and no cycles to detect.
- An unused variable is allowed and kept (it may be about to be used); it is not an error.
- **The lexer is unchanged** (§2.15): `$thr`, `=` and the operators are ordinary words, split as today at whitespace
  and `( ) [ ] , ; "`. So a declaration needs spaces around `=` (`$thr = 95%`); `$thr=95%` is refused with a message
  that shows the spaced form. `$` stays a word character, which is what lets `$a-$b`, `$p-auto`, `+$p` and
  `$min..$max` stay one word each, split by the parser like today's `45-75%` and `2m..4m`.
- **Rejected:** variables scoped to a repeat (`3x[$on = 30s](…)`): shadowing rules, and the outline would have to
  show scopes, for a need nobody has stated. **Rejected:** declarations anywhere (canonical printing would move them,
  so the printed text would differ from the typed text more than it does today). **Rejected:** names without a sigil
  (`thr = 95%`): every future keyword would break someone's workout.

### 2.7 Types

A variable's type is the unit of its value; there are no type annotations.

| Type | Literal forms | Used where |
|---|---|---|
| % FTP | `95%` | a `%` target, range ends (`$lo-$hi`, `$lo-80%`), ramp ends (`ramp $a-$b`, `ramp $a-auto`), a progression (`[+$p]`) |
| watts | `250w` | a `w` target, a progression (`[+$p]`) |
| duration | `4m`, `1m30s` | a time step, `hr<` bounds (`hr<118 $min..$max`) |
| number | `3`, `0.5` | a repeat count (`$n x(…)`), and a factor in arithmetic |

Cadence, heart rate and kJ take no variables (`90rpm`, `140bpm`, `10kj` stay literal). They can be added as types
later without changing anything here; nobody has asked for them.

### 2.8 Expressions

Arithmetic is allowed **in declarations only**:

```ebnf
expr   = term { ( "+" | "-" ) term } ;           (* binary operators stand alone: spaces around them *)
term   = factor { ( "*" | "/" ) factor } ;
factor = [ "-" ] ( literal | VAR | "(" expr ")" ) ;  (* unary minus is attached: -5w, -$x *)
```

Typing: `+` and `-` take two values of the same type; `*` takes a value and a number (either order); `/` divides a
value by a number. A % FTP value and a watts value never combine (% FTP already means "a fraction of FTP"; converting
would freeze the workout to one FTP). `$thr - 10%` is therefore **percentage points**: 95 % − 10 % = 85 %. The
relative meaning (85.5 %) is written `$thr * 0.9`. Rejected: reading `- 10%` as relative, which would make `-` mean
one thing for % FTP and another for watts and durations, and would leave no way to write points.

**Rounding.** A resolved power is rounded to what a literal of its type can say, with the printer's rule (ties towards
+∞, DSL.md "Rust port notes"): watts to whole watts (`250w / 3` → 83 W, as `INT "w"` can only write whole watts), %
FTP to three decimals (`100% / 3` → 33.333 %). Durations and counts are **not** rounded: they move the timeline and
the structure, so a result that is not whole is an error (below). Because every resolved value is exactly a literal
the DSL can write, "Inline" (§2.12) writes that literal and the result compiles identically, and printing the inlined
text and parsing it back gives the same values.

**Checks**, made by the parser at each use, with the use's span (§2.14), and again by `resolve_vars` for a tree that
did not come from text:

- a power must be ≥ 0 (`$e = 40% - 50%` used as `5m $e` is −10 %: an error, never the compiler's clamp to 0 W);
  a range's ends must be ordered (today's `ReversedPowerRange`); plausibility ≤ 400 % FTP as today;
- a duration must be a positive whole number of seconds (`4m / 7` = 34.286 s and `1m - 2m` are errors);
- a repeat count must be a whole number ≥ 1 (`$n = 7 / 2` used as a count is an error);
- a progression may be negative (it is today: `[-10w]`).

The compiler's existing `.max(0.)` in `target_watts` still applies to targets that a **negative progression** drives
below zero; that is today's behaviour (reference parity), not something variables add, and it is tracked in #696.

Operators need spaces (`$work - 30s`), because `-` inside a word is already a range (`45-75%`) and a sign (`+5w`);
`$work-30s` is an error that says so.

- **Chosen** because the owner's examples are relations between values: rest = half the work, tempo = threshold minus
  10 %. Plain substitution cannot keep those in step.
- **Rejected:** plain substitution only (above). **Rejected:** expressions inside blocks (`4m $thr * 0.9`): the block
  grammar gains operators that clash with ranges and signs, and the outline could no longer show a block's target as a
  short phrase. A derived value gets a name instead.
- **Rejected:** a built-in `ftp` (`$x = ftp * 0.9`): it is `90%`, and as watts it would no longer follow the rider's FTP.
- **Library (owner rule 2026-11-14):** searched crates.io (2026-11-22) for an evaluator: `evalexpr` 14.0.0, `fasteval`
  0.2.4, `meval` 0.2.0. All evaluate untyped floating-point expressions; none has units, none reports UTF-16 spans in
  our `DslError` form, and they bring their own lexers for a grammar of four operators that must share this DSL's lexer
  and error spans. So the expression grammar extends the existing hand-written parser (`dsl.rs`), with the property
  tests of §5 as the justification the rule asks for.

### 2.9 Uses

A use is `$name` where a number with that unit would stand, plus the forms of §2.1 and §2.7:

```text
$thr = 95%
$on = 4m
$rest = $on / 2
$sets = 3

"Warm-up" 10m ramp 45%-auto
"Threshold" $sets x[!last]("On" $on $thr 90rpm, "Off" $rest 55%)
"Cool-down" 5m ramp auto-40%
```

A repeat count is `$sets x(…)` with a space before `x` (`$setsx` would be a name). A power variable used alone as a
target becomes a `%` or `w` target by its type.

**Every position a variable may take** (this completes DSL.md's grammar; a form is one word token unless the rule
shows a space):

```ebnf
end       = DURATION | VAR                                   (* VAR: duration *)
          | NUMBER "kj" | "lap"
          | "hr<" INT  bound ".." bound ;                    (* two words, as today: hr<118 $min..4m *)
bound     = DURATION | VAR ;                                 (* VAR: duration *)
target    = NUMBER "%" | INT "w" | VAR                       (* VAR: % FTP gives a % target, watts a w target *)
          | range | "ramp" ramp_ends | INT "bpm" | "free" ;  (* ramp_ends: §2.1 *)
range     = NUMBER "-" NUMBER "%"                            (* today: 50-75% *)
          | pend "-" pend ;                                  (* new: at least one VAR; $lo-$hi, $lo-80%, 60%-$hi *)
pend      = NUMBER "%" | VAR ;                               (* VAR: % FTP; a range is % FTP only, as today *)
repeat    = COUNT "x" [ options ] "(" workout ")"            (* today: 3x(…), one word "3x" *)
          | VAR " " "x" [ options ] "(" workout ")" ;        (* new: $n x(…), two words; VAR: number *)
option    = SIGNED "w" | SIGNED "%" | "!last"                (* today *)
          | "+" VAR ;                                        (* new: +$p, one word; VAR: watts or % FTP *)
```

Rules the grammar does not show:

- In a new-form range, as in a ramp, every number carries its own `%` (`$lo-80` is refused with the fix).
- A progression variable is always written `+$p`; a falling progression declares a negative value (`$drop = -5w`,
  then `[+$drop]`). `[-$p]` and `[$p]` are refused with a message saying so, so a progression field holds a plain
  reference and never a negated one.
- A name cannot contain `-`, `.` or `+`, so splitting a word at `-`, `..` or a leading `+` is unambiguous.

### 2.10 Model

Names live in the tree (fact in §1), so a numeric field the user can bind holds a number or a reference:

```rust
/// A literal or a variable reference; serialises as `95` or `{ "var": "thr" }` (serde untagged).
pub enum Num { Lit(f64), Var { var: String } }

pub struct VarDecl { pub name: String, pub expr: Expr }          // value is derived, never stored
pub enum Expr { Lit { value: f64, unit: Unit }, Var { name: String }, Neg(Box<Expr>), Bin { op: Op, l: Box<Expr>, r: Box<Expr> } }
pub struct Workout { /* id, name, ftp, nodes as today */ #[serde(default)] pub vars: Vec<VarDecl> }
```

`Num` replaces `f64` in: `StepEnd::Time.s`, `StepEnd::HrRecovery.{min_s, max_s}`, every `PowerTarget` power field
(including `AutoRamp`), `RepeatNode.count` and `Progression.{watts_per_rep, pct_per_rep}`. `parse_dsl` returns the
declarations with the nodes (a `DslDoc { vars, nodes }`), and `print_dsl` takes them.

`resolve_vars(&Workout) -> Result<Workout, ResolveError>` evaluates the declarations top to bottom and returns a tree
with only literals. Text never reaches it unchecked (the parser reports §2.14's errors with spans); `ResolveError` is a
typed refusal for a tree built or deserialised directly (an unknown name, a type that does not match the field), which
`compile_workout` returns as a new `CompileError` kind. `compile_workout` calls `resolve_vars` first and works on the
result exactly as today. The ride snapshot stores the workout as authored (names kept, so it reprints as written) beside the compiled blocks it rode
(values).

- **Rejected:** a second, source-level node tree beside the resolved one (`SourceNode` mirroring `WorkoutNode`):
  every tree helper (`re_id`, `find_node`, the editor's `removeNode`, `wrapNode`, …) would exist twice, and the editor
  would have to keep two trees in step. **Rejected:** the resolved value and the name side by side in each field: two
  copies of one fact that an edit can let drift apart.
- **Cost, stated:** the TypeScript bindings change from `number` to `number | { var: string }` in those fields, so app
  code that reads them (outline labels, the editor model, the mocks) must handle a reference. Existing JSON
  deserialises unchanged, because a bare number is `Num::Lit`.

### 2.11 Canonical printing

- Declarations first, one per line, `$name = expr`, one space around `=` and every binary operator, parentheses only
  where precedence needs them (`$a - ($b - $c)` keeps them, `($a * 2) + $b` prints `$a * 2 + $b`), literals in today's
  canonical forms (`90s` → `1m30s`, `%` to three decimals).
- A blank line separates the declarations from the blocks; blocks print as today.
- Uses print the name, never the value: `print_dsl(parse_dsl(t))` keeps every `$name`.
- `auto` prints lower case in its position.
- A workout without declarations or `auto` prints **byte-identically** to today (§2.15).

### 2.12 The editor (#111, #653)

- **Outline:** declarations are the first rows, under a "Variables" heading, each "$thr = 95 %" (an expression shows
  its value too: "$rest = $on / 2 · 2:00"). A block row shows the name with its value: "On · $on (4:00) · $thr (95 %)".
- **Editing a variable** (row action or the text) changes every use: the editor builds the new declaration list,
  prints the workout with `print_dsl`, parses it again and commits that with `applied`, one undo step (a structural
  edit, DESIGN §5.7). The re-parse means a type change that breaks a use is a draft error shown like any other, never
  a half-applied tree.
- **Editing a bound value in one block** (outline, or an AI rewrite on a selection) writes a literal into that block:
  it unbinds, and the outline says "no longer uses $thr" in the action's result. It never edits the declaration,
  because that would change blocks outside the selection; the AI's content-hash check (#1) covers only the selected
  node, and other blocks changing under it would break that guarantee.
- **An AI rewrite that writes `$name`** (the planner sees the declarations with the selection, #29): a reference to a
  declared variable of the right type is kept, so the rewritten block stays bound. The rewrite cannot add, remove or
  change a declaration; a reference to an undeclared name or of the wrong type makes the proposal invalid, and it is
  not offered, like a proposal whose text does not parse (the planner's retry is #29's). The proposal's diff shows
  the reference as written, and the change summary gives its value ("$thr (95 %)").
- **Duplicate / insert after** keep references. **Delete** of a block keeps declarations. **Deleting a variable** that
  is still used is refused with the number of uses ("$thr is used by 3 blocks"); an "Inline" action replaces each use
  by the value and then deletes it, one undo step.
- **Undo groups** are unchanged: typing a declaration is a typing burst like any other text; each outline action is
  one step.
- DESIGN §5.7 gets these rules in the implementation PR (with design-harness fixtures), as #653 already requires for
  its actions.

### 2.13 Coach and rider adjustments

Order: **variables → pins → compile → ride-time adjustments.** The rider's ± and the coach's ±% (DESIGN §7.1) apply to
the compiled blocks during the ride exactly as today. A pin is a planning relation: it is worked out at compile time
and is not re-worked out when the coach changes a block's target. Where the coach's change covers both a pinned ramp
and its neighbour, the joint stays continuous; where it covers only one, it moves that block like any other.

- **Rejected:** re-pinning during the ride. The engine would need the DSL's neighbour rules, and the coach's
  "remaining work by at most −5 %" bound (§7.1) is judged on blocks; it would silently widen if a change to one block
  dragged its neighbours with it.
- The rider's live edits (DESIGN §17: skip, restore, extend, repeat once, end set) work the same way: §17.1's overlay
  never changes a block's target, so skipping the block a ramp is pinned to leaves the ramp's end where it was
  planned. A rider who skips the set after a warm-up rides into whatever comes next with a step, as with an explicit
  ramp today. `repeat_once` is the exception, because §17.1 promises an equality that pins would otherwise break;
  §2.13.1 amends it. (`repeat_once` is never coach-initiated: §17.2, `coach_cannot_extend`.)
- A coach −5 % is never written back into a variable: variables are the plan, and §17 already keeps rider edits and
  coach changes orthogonal to it.

#### 2.13.1 Amendment to DESIGN §17.1 step 2 (`repeat_once` with pins)

§17.1 step 2 promises that adding `n` reps "equals compiling the same workout with `count + n` and the progression
held". With pins it would not: in `10m 50%, 3x(4m 100%, 2m ramp auto-auto), 5m 40%` the last iteration's ramp
compiles 100 → 40 % (it leads into the cool-down) while the earlier ones compile 100 → 100 %. Copying the last
iteration as it is would put a 100 → 40 % ramp before the added 4 min at 100 % (a step up of 60 % FTP) and end the
copy with 100 → 40 % again; compiling with `count = 4` gives 100 → 100 % in iteration 2 and 100 → 40 % only in 3.

**Amended step 2:** after the copies are inserted, `effective` re-resolves the pinned ends **at the seams** the
insertion made: the original last iteration's blocks whose pinned end now meets the first copy, each copy's pinned
start, and the last copy's pinned end that meets the block after the group. Re-resolving is the compiler's own rule
applied to the effective sequence: a pinned start takes the previous effective block's `power_to`, a pinned end the
next one's `power_from` (`FlatBlock.pinned_from` / `pinned_to`, §2.5, say which ends are pinned; §2.3 guarantees the
neighbour's end is never pinned, so this is one look-up per end, no chain). `effective` stays a pure function in
`crank-domain`, sharing the function the compiler uses.

**The cursor.** A block that is current or done never changes (the engine invariant of §17.1). The `repeat_once` edit
row records, when it is accepted, the seam blocks that had already started (`frozen: [BlockId]`), and `effective`
leaves those as they were. So a rider who asks for one more rep while already in the last iteration's ramp keeps that
ramp as it is and meets a step into the added rep: the one case where the equality does not hold, stated in the
amended text. The undo of a `repeat_once` removes its row from the fold, so the seam ramps return to their compiled
targets.

**Amended equality and invariant:** the result equals compiling with `count + n`, progression held, **pinned ends
included**, except seam blocks frozen at accept; the overlay invariant "no block's target is changed" becomes "no
block's target is changed, except a pinned end at a `repeat_once` seam that had not started", and step 5's "`power_from/to`
untouched" gets the same exception. These texts go into DESIGN §17.1 in the implementation PR.

- **Rejected:** leave the copy as it is (no re-pinning): the rider gets a step of up to the whole work-to-rest
  difference exactly where the writer pinned for continuity, and §17.1's equality silently becomes false.
- **Rejected:** re-pin after every overlay edit (skip, end set too): a skip next to the current block would change
  the current block's target, which §17.1's engine invariant forbids, and Skip's meaning ("drop a block") would grow
  a side effect on its neighbours.

### 2.14 Parse errors

All carry `offset` / `length` (UTF-16) of the token named in the message, and a typed `DslErrorKind`:

| Text | Message | Highlighted |
|---|---|---|
| `4m $thr` (not declared) | Unknown variable '$thr' — declare it before the first block, e.g. $thr = 95% | `$thr` |
| `$a = $b * 2` then `$b = 1m` | Unknown variable '$b' — a declaration can use only variables declared above it | `$b` |
| `3m 60%` then `$x = 1m` | Variables are declared before the first block | `$x` |
| `$thr = 95%` twice | '$thr' is already declared | the second `$thr` |
| `$d = 4m` and `10m $d` | '$d' is a duration (4m); a power target is expected here, e.g. 95% or 250w | `$d` |
| `$p = 250w` and `ramp $p-auto` | '$p' is in watts; a ramp is in % FTP | `$p-auto` |
| `$a = 95% + 250w` | Cannot add % FTP and watts | `+` |
| `$a = 4m * 2m` | A duration can be multiplied only by a plain number | `*` |
| `$a = 1m / 0` | Division by zero | `/` |
| `$n = 7 / 2` and `$n x(…)` | Repeat count '$n' is 3.5; it must be a whole number ≥ 1 | `$n` |
| `$d = 4m / 7` and `$d 90%` | '$d' is 34.286 s; a duration must be whole seconds | `$d` |
| `$a = $b-30s` | Put spaces around '-' in an expression: $b - 30s | `$b-30s` |
| `$1x = 3` | A variable name starts with a letter, e.g. $thr, $rest_2 | `$1x` |
| `ramp 45-auto` | Write the % on the number in a ramp with 'auto', e.g. ramp 45%-auto | `45-auto` |
| `ramp $a-75` / `$lo-80` | Write the % on the number next to a variable, e.g. ramp $a-75% | `$a-75` / `$lo-80` |
| `ramp 45%-75%` | Write a ramp between two numbers as ramp 45-75% | `45%-75%` |
| `$p = 250w` and `5m $p-80%` | '$p' is in watts; a range is in % FTP | `$p-80%` |
| `$e = 40% - 50%` and `5m $e` | '$e' is -10 % FTP; a power must be 0 or more | `$e` |
| `$lo = 80%`, `$hi = 60%` and `5m $lo-$hi` | Range low end is above high end (use 'ramp 80-60%' for a descending ramp) | `$lo-$hi` |
| `$d = 1m - 2m` and `$d 50%` | '$d' is -60 s; a duration must be more than 0 | `$d` |
| `$n = 0` and `$n x(…)` | Repeat count '$n' is 0; it must be a whole number ≥ 1 | `$n` |
| `$thr=95%` | Put spaces around '=': $thr = 95% | `$thr=95%` |
| `$p = 5w` and `3x[-$p](…)` | Write a falling progression as a negative variable: $p = -5w, then [+$p] | `-$p` |
| `5m auto` | 'auto' is only a ramp end, e.g. ramp auto-75% | `auto` |

Bounds, for the same stack-safety reasons as `MAX_NESTING_DEPTH`: at most 100 declarations, expression nesting at most
32, names at most 32 characters; each has its own error kind with the limit as a number.

### 2.15 Backward compatibility

- **Every text that parses today parses to the same tree and prints byte-identically.** The lexer does not change,
  so every text gives the same tokens with the same spans. The parser gives a new meaning only to words that contain
  `$` or the keyword `auto`, and to the standalone words `=`, `*`, `/`, `+`, `-` inside a declaration. A valid text
  today contains none of these outside a label or a comment: every word must be a duration, number with a unit,
  keyword (`ramp`, `lap`, `free`, `hr<…`, `!last`, `!lower`, `!cap`, `!coach`), signed option or repeat count, and
  none of those can hold `$` or `auto` or be a bare operator, so each such word is a parse error today. The four
  forms of today's grammar (`45-75%` ranges and ramps, `3x`, `[+5w]`, `2m..4m`) are matched first and unchanged; the
  new forms are tried only on words they refuse.
- **The grade-profile grammar** (routes, DSL.md "Grade profiles", ADR-0005 §2.2.5) shares the lexer and nothing
  else. Its tokens and error spans are unchanged, and it gains no variables and no `auto`: a `$x` or `auto` word in a
  grade profile is refused with today's message ("Expected a distance in km … got '$x'") at today's span.
- Every stored `Workout` and compiled snapshot deserialises unchanged: `AutoRamp` is a new variant, `vars` defaults
  to empty, `Num` accepts a bare number, and the new `FlatBlock` flags are omitted when false.
- The engine, the recorder and the ride's data path do not change; they read compiled blocks.

## 3. Alternatives considered

Each decision above lists its rejected alternatives in place. The larger ones, together:

- **Resolve everything before the compiler** (pins on the tree): cannot express per-iteration neighbours (§2.2).
- **Warnings with fallbacks** for unpinnable neighbours: silent behaviour in the ride (§2.3).
- **Inline expressions in blocks**: grammar clashes, an outline that cannot phrase a target (§2.8).
- **A separate source tree**: every tree helper twice (§2.10).
- **Re-pinning during the ride**: DSL rules in the engine, widened coach bounds (§2.13).

## 4. Consequences

- A warm-up or cool-down written once stays joined to the set it leads into, through edits and progression.
- A workout can be retuned from one line (`$thr`), and a library workout can say what it is about.
- The bindings change in several numeric fields (`number | { var }`); the app's outline, editor model and mocks are
  updated in the implementation PR, the mocks answering only from the core's own answers (`editor.json`, #653).
- DESIGN §17.1 is amended for `repeat_once` with pins (§2.13.1): the overlay re-resolves pinned ends at the seams, and
  its invariant gains that one exception.
- Two new features of the compiler (`resolve_vars`, pin resolution) and four new `ValidationKind`s; the
  `ValidationKind` list and the TS bindings change accordingly.

## 5. Test plan

Rust, in `crates/crank-domain/tests/` beside the existing DSL and compile tests; every property at the default 256
cases.

**Round trip (properties).**
- `parse(print(d)) == d` (ids aside) and `print` idempotent, over generated documents that now include declarations,
  expressions of every operator and nesting up to the bound, bound fields of every position in §2.7, and auto ramps
  of all three forms. The generator draws declaration-before-use so every case is valid.
- The existing round-trip property and every golden text of #68 and the built-ins run **unchanged** and still print
  byte-identically (backward compatibility, §2.15).
- Arbitrary strings over the extended alphabet (`$ = * / auto` added) complete with `Ok` or a typed error.

**Resolution (properties).**
- Inlining: `compile(d) == compile(inline(d))`, where `inline` replaces each use by its value as a literal.
- Pin equivalence: for a top-level pinned ramp outside any progressing repeat, `compile` equals the compile of the
  same workout with the pinned end replaced by the neighbour's resolved value, except the `pinned_*` flags.
- Continuity: for every resolved pin, `previous.power_to == ramp.power_from` (and the mirror for a pinned end),
  exactly, on every occurrence.
- The tiling property of #69 holds with auto ramps and variables in the generator.

**Compile goldens, one per neighbour case of §2.2–§2.4:** steady step in % and in W; range (midpoint); explicit
ramp; an `auto` start after an explicit ramp's number end (`ramp 40%-80%, ramp auto-60%, ramp auto-90%`); a bridge
between two steady blocks (`5m 50%, 30s ramp auto-auto, 10m 95%`); repeat after (iteration 0, first child); repeat before with `!last` (second-last child) and
with `[+5w]` (last iteration's progressed power); auto ramp as a repeat's first child (iteration 0 vs. later); auto
ramp as a repeat's last child; HR recovery neighbour (planned power); progression not applied to the auto ramp;
phase from resolved ends, with and without a "warm" label.

**Facing and no-neighbour goldens (errors):** `ramp 40%-auto, ramp auto-auto, 10m 95%` (facing, reported once on the
first ramp, naming the second); `3x(1m ramp auto-auto)` after a steady block (pinned to itself across the repeat,
reported once, not per iteration); `3x(1m ramp auto-80%, 2m ramp 50%-auto)` (facing across the iteration boundary,
two nodes); first-block pinned start and last-block pinned end.

**Rounding and inlining:** `250w / 3` resolves to 83 W and `100% / 3` to 33.333 %; for generated documents,
`inline(d)` prints to text that parses to the same values, and `compile(d) == compile(inline(d))` exactly.

**§2.13.1 overlay (in the `workout::overlay` property tests of §17.7):** for generated workouts with pinned ramps
inside and around repeats, `effective(compiled, repeat_once × n)` equals `compile` with `count + n` and progression
held, pinned ends included; with a seam block frozen, only that block differs; undo restores the compiled targets;
the engine invariant (nothing at or before the cursor changes) holds with pins in the generator.

**Error cases:** each row of §2.3 (issue kind, severity, `node_id`, message) and of §2.14 (kind, message, offset and
length), and each bound of §2.14 at the limit (succeeds) and one past it (refused).

**App:** the editor model's tests for variable edit (one undo step, re-parse), unbind-on-block-edit, refused delete
of a used variable, Inline; the outline labels; the mocks' drafts added to `editor.json`.

## 6. Open questions

None blocking. The decisions marked for the owner above are the questions; anything the review adds is filed as an
issue and linked here before this ADR is accepted.

| Question | Issue | Milestone |
|---|---|---|

## 7. References

Checked 2026-11-22.

- intervals.icu, Workout Builder Syntax Quick Guide (official forum): https://forum.intervals.icu/t/workout-builder-syntax-quick-guide/123701
- intervals.icu, Workout builder thread: https://forum.intervals.icu/t/workout-builder/1163
- Zwift workout file reference (community, `descriptions.yaml`): https://www.github.com/h4l/zwift-workout-file-reference/blob/master/descriptions.yaml
- Zwift Insider, Manually Editing Zwift Workouts: https://zwiftinsider.com/manually-editing-zwift-workouts
- R. Saarsoo, "ZWO sucks" (format inconsistencies, PowerLow/PowerHigh): https://nene.github.io/2021/01/14/zwo-sucks
- TrainerRoad, Creating a Workout from an ERG or MRC File: https://support.trainerroad.com/hc/en-us/articles/201944204-Creating-a-Workout-from-an-ERG-or-MRC-File
- MRC_creator (ramp "from previous value"): https://github.com/helderlopes/MRC_creator
- crates.io: `evalexpr` 14.0.0, `fasteval` 0.2.4, `meval` 0.2.0 (`cargo search`, 2026-11-22)
- In repo: `docs/DSL.md`; `crates/crank-domain/src/workout/{model,compile,compiled,dsl}.rs`; `app/src/editor/model.ts`;
  DESIGN §5.7, §7.1, §17
