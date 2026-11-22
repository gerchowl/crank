# ADR-0008: Workout DSL: auto ramps pinned to neighbouring blocks, and variables

- **Status:** Proposed (awaiting owner review; no code until accepted)
- **Date:** 2026-11-22
- **Issue:** #694
- **Related:** `docs/DSL.md` (the production grammar, #68), the compiler (#69, `crates/crank-domain/src/workout/compile.rs`),
  DESIGN §5.7 (editor, undo groups), §7.1 (coach bounds), §17 (live workout edits), #111 (editor), #653 (outline edits),
  #110 (stored library), ADR-0001 §3–4 (domain contract, bindings)

## For the owner

This proposal amends the workout DSL. Nothing is implemented. Each line below is a decision that needs a yes or a
no; the section in brackets has the reasons and the rejected alternatives.

1. **Pinned ramp syntax** `ramp auto-75%`, `ramp 45%-auto`, `ramp auto-auto`: `auto` takes the place of a number,
   and in a form with `auto` the `%` sits on the number (§2.1).
2. **A pinned end takes the power of the block next to it on the ridden timeline**: the end of the block before,
   the start of the block after, with repeats, progression and `!last` applied, worked out for each occurrence
   (§2.2).
3. **No guessing**: pinning to nothing (first or last block), to a free ride, to a heart-rate target, or two ramps
   pinned to each other is an error that blocks Ride this, not a warning with a fallback (§2.3).
4. **A pinned ramp is never progressed** by `[+5w]` / `[+2%]`; its pinned end already follows the progressed
   neighbour (§2.4).
5. **Variables** are `$name = value` lines before the first block, one scope for the whole workout (§2.6).
6. **Arithmetic only in declarations** (`$rest = $work / 2`, `$tempo = $thr - 10%`); a block uses a variable by
   name only (`4m $thr`) (§2.8).
7. **The coach's ±% and the rider's ± apply after** variables and pins are resolved, as they apply today to the
   compiled blocks, and so do the rider's live edits (skip, extend, repeat once); a pin is not re-worked out during
   the ride (§2.13).
8. **An edit to one block never edits a variable**: changing a block's bound value from the outline or by an AI
   rewrite writes a number into that block (it unbinds); a variable is changed only through its own row or the
   text (§2.12).
9. **The tree keeps names**: a numeric field holds a number or `{ "var": "thr" }`, and the workout holds its
   declarations, rather than a second source tree beside the resolved one (§2.10).

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
target   = … | "ramp" ramp_ends ;
ramp_ends = NUMBER "-" NUMBER "%"              (* today: ramp 45-75% *)
          | end_pct "-" "auto"                 (* ramp 45%-auto: end pinned *)
          | "auto" "-" end_pct                 (* ramp auto-75%: start pinned *)
          | "auto" "-" "auto" ;                (* both pinned: a bridge *)
end_pct  = NUMBER "%" | VAR ;                  (* VAR: a % FTP variable, §2.9 *)
```

`auto` is a keyword (case-insensitive, printed lower case) valid only as a ramp end. As today, `ramp 45%-auto` is
one word token, split at `-`.

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
| an `auto` end meets an `auto` start (A's end pinned, B right after with its start pinned) | `AutoRampFacing` | Two ramps are pinned to each other — give one of these ends a number |
| `AutoRamp` with no `auto` end (only by deserialising, §2.1) | `AutoRampNotPinned` | Ramp has no 'auto' end |

There is no SIM or other unpowered block in the workout grammar (routes are a sibling grammar, ADR-0005 §2.2.5); if
one is ever added it falls under `AutoRampUnplannedNeighbour` by the same reasoning as `free`.

`auto-auto` resolves both ends independently; it is facing-pinned only if a neighbour faces it with its own `auto`.
Chains such as `ramp 40%-auto, ramp auto-auto, 10m 95%` resolve in timeline order because each pinned end reads a
neighbour end that is not itself pinned towards it; the facing pair is the only cycle a timeline can contain.

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
- `$` and `=` become token separators in the lexer. Neither is accepted anywhere today outside labels, so no valid
  text changes meaning.
- **Rejected:** variables scoped to a repeat (`3x[$on = 30s](…)`): shadowing rules, and the outline would have to
  show scopes, for a need nobody has stated. **Rejected:** declarations anywhere (canonical printing would move them,
  so the printed text would differ from the typed text more than it does today). **Rejected:** names without a sigil
  (`thr = 95%`): every future keyword would break someone's workout.

### 2.7 Types

A variable's type is the unit of its value; there are no type annotations.

| Type | Literal forms | Used where |
|---|---|---|
| % FTP | `95%` | a `%` target, range ends, ramp ends (`ramp $a-$b`, `ramp $a-auto`), a `[+…]` progression |
| watts | `250w` | a `w` target, a `[+…]` progression |
| duration | `4m`, `1m30s` | a time step, `hr<` bounds (`hr<118 $min..$max`) |
| number | `3`, `0.5` | a repeat count (`$n x(…)`), and a factor in arithmetic |

Cadence, heart rate and kJ take no variables (`90rpm`, `140bpm`, `10kj` stay literal). They can be added as types
later without changing anything here; nobody has asked for them.

### 2.8 Expressions

Arithmetic is allowed **in declarations only**:

```ebnf
expr   = term { ( "+" | "-" ) term } ;           (* binary operators stand alone: spaces around them *)
term   = factor { ( "*" | "/" ) factor } ;
factor = [ "-" ] ( literal | VAR | "(" expr ")" ) ;
```

Typing: `+` and `-` take two values of the same type; `*` takes a value and a number (either order); `/` divides a
value by a number. A % FTP value and a watts value never combine (% FTP already means "a fraction of FTP"; converting
would freeze the workout to one FTP). Results are checked where used:

- a repeat count must be a whole number ≥ 1 (`$n = 7 / 2` used as a count is an error);
- a duration must be a positive whole number of seconds (`4m / 7` = 34.286 s is an error);
- powers follow today's rules (plausibility ≤ 400 %, `ExcessivePower`, and so on), applied to the resolved value.

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
`compile_workout` returns as a new `CompileError` kind. It calls `resolve_vars` first and works on the result exactly
as today; `compile_workout` calls it first and works on the result exactly as today. The ride
snapshot stores the workout as authored (names kept, so it reprints as written) beside the compiled blocks it rode
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
  ramp today. `repeat_once` copies the last iteration with its targets, pinned ends included, as resolved.
- A coach −5 % is never written back into a variable: variables are the plan, and §17 already keeps rider edits and
  coach changes orthogonal to it.

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
| `5m auto` | 'auto' is only a ramp end, e.g. ramp auto-75% | `auto` |

Bounds, for the same stack-safety reasons as `MAX_NESTING_DEPTH`: at most 100 declarations, expression nesting at most
32, names at most 32 characters; each has its own error kind with the limit as a number.

### 2.15 Backward compatibility

- Every text that parses today parses to the same tree and prints byte-identically: the new syntax uses `$`, `=` and
  the word `auto`, none of which a valid text can contain today outside a label.
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
ramp; auto-ramp chain; repeat after (iteration 0, first child); repeat before with `!last` (second-last child) and
with `[+5w]` (last iteration's progressed power); auto ramp as a repeat's first child (iteration 0 vs. later); auto
ramp as a repeat's last child; HR recovery neighbour (planned power); progression not applied to the auto ramp;
phase from resolved ends, with and without a "warm" label.

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
