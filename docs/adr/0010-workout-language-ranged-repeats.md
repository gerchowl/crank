# ADR-0010: Workout language: keep the declarative DSL (no script layer); ranged repeats that end on a ride-time condition

- **Status:** Proposed
- **Date:** 2026-11-27
- **Issue:** #736
- **Related:** `docs/DSL.md` (grammar), ADR-0008 (variables, pinned ramps; accepted, not implemented), ADR-0009 (analysis,
  planned vs ridden), ADR-0002 §7 (ERG policy), DESIGN §17 (live workout edits, the plan overlay; #240), #224 (engine end
  rules for open-ended blocks), #653/#731 (outline edits), #699 (generation), #732 (lint, completion), #627 (ERG
  reachability warning), #257 (FIT export)

## For the owner

Each line needs a yes or a no; the section in brackets has the reasons and the rejected alternatives.

1. **Keep the declarative DSL** as the one workout language: stored, edited, rewritten, compiled and ridden. **No
   script layer (Starlark or another)** now, neither as a replacement nor as an optional generator in front of it
   (§2.1). The conditions you asked for act during the ride, and a script that runs before the ride cannot express them.
   They belong in the declarative tree in every option (§1.2).
2. **ADR-0008 stays as accepted**, unchanged, and is implemented first. Its `Num` (literal or `$variable`) also covers
   the new fields below (§2.2).
3. **Ranged repeat:** `3..10x[hr>=170](30s 120%, 15s 50%)` means at least 3 and at most 10 reps, ending after the rep in
   which HR reaches 170 bpm. Your example reads:
   `3x("Pump" 3..10x[hr>=170, !last](30s 120%, 15s 50%), "Long rest" 4m 40%)` (§2.3).
4. **Bounds are mandatory**: a rep range needs a condition, a condition needs a rep range, and `min < max`. Every
   workout keeps a finite, known length range. That one is 18:00 to 33:45 (§2.3, §2.6).
5. **Triggers in v1:** heart rate only, `hr>=N` (rises to) and `hr<N` (falls below), in bpm. Each must hold for **5
   consecutive valid seconds**. Cadence, power and time-in-zone triggers are not in v1 (§2.4). Say yes to item 12 if you
   want cadence too.
6. **When a set ends:** the condition is watched during the whole set. Once it has held, the set ends at the end of that
   rep, and never before rep `min`. With `!last`, the short rest of the final rep is dropped and the ride goes straight
   to the block that follows (§2.5).
7. **HR lost** (strap gone, or no valid HR for 10 s at a rep's end): the set ends after rep `min`, or after the current
   rep if `min` has passed. A visible note says so. A lost signal always resolves to the bound with **less work**
   (§2.5). For a ranged repeat that is `min`. For today's `hr<` recovery step it is the longer rest (ADR-0001 §5),
   already decided.
8. **Compile at the maximum:** the compiler expands every ranged repeat to its `max` reps, so every gate (errors, the
   health cap, W′bal, ERG reachability #627, expansion limits) runs on the worst case. During the ride an early end
   **suppresses** the remaining reps through DESIGN §17's overlay (a new kind, `until_met`, actor `plan`). The ERG
   control law does not change (§2.6, §2.7).
9. **Ranges, not guesses**, before the ride: the Ready sheet and the library show the length and TSS as ranges
   (for example "18:00–33:45 · TSS 41–63"). Their ends come from compiling at all-`min` and at all-`max` (§2.8).
10. **Rider and coach:** End set, Skip and suppress work on a ranged set as on any repeat. `repeat_once` is refused on
    it (`ranged_repeat`), because the condition owns its count. The coach's ±% applies to targets as today. The coach
    never changes a range (§2.9).
11. **Analysis records each set's outcome**: reps ridden, why the set ended (`condition_met` with time and value,
    `max_reached`, `signal_lost`, `rider_end_set`) and the trigger value. Reps you didn't need are not counted as
    skipped or missed. "Reps to reach the trigger" becomes a set statistic, shown with no verdict (§2.10).
12. *(Optional, recommended no for v1)* a cadence trigger `cad<N` ("until I can't hold 80 rpm"), held 10 s and watched
    only in work blocks after ERG settles (§2.4).

## 1. Context

The owner asked (2026-11-27, #736):

> should the workout 'text' become a starlark or sth? the reduced but complete coding? before we do some hand-rolled
> new language stuff? in the end it needs to spit out a correct 'workout' by our gates. To also allow fancier workout
> structures, e.g. intervals of fixed time with short rests that pump the HR up, and then either on total reps or e.g.
> a HR threshold go into a longer rest block before repeating.

### 1.1 What exists (dev at `0438c9b1`)

- **The DSL is declarative and small.** `parse_dsl` / `print_dsl` (`crates/crank-domain/src/workout/dsl.rs`) round-trip:
  `parse(print(t)) == t` (ids aside), printing idempotent, property-tested (DSL.md). The grammar has steps with an end
  (time, kJ, `lap`, `hr<N min..max`), a target and cadence, plus repeats with progression and `!last`. Errors are typed
  (`DslErrorKind`) with UTF-16 spans. The parser bounds nesting at 32 levels.
- **The tree is the stored form, the text is derived** (ADR-0008 §1). `Workout { id, name, ftp, nodes }` is what the
  library holds and what a ride snapshots. Every outline edit (#653, #731) changes the tree and commits it with its
  canonical text.
- **The compiler flattens before it times.** `compile_workout` (`compile.rs`) expands repeats into `FlatBlock`s
  (`compiled.rs`). Each block has `path`, `iteration`, `start_s`, `dur_s`, `power_from/to`, `phase` and `dur_range_s`
  for open-ended blocks. The compiler also produces `total_s`, `total_range_s`, TSS, IF, time in zone and typed
  `ValidationIssue`s. Hard limits are `CompileError::ExpansionLimit` (100 000 node visits) and `TraceLimit` (604 800
  s). These are the "gates".
- **The engine runs compiled blocks only.** Today it advances when `block_elapsed >= block_ticks(dur_s)`
  (`crates/crank-engine/src/engine.rs`, the block sequencer). **The `hr<` end is not evaluated during the ride yet:** a
  recovery block lasts its compiled estimate. That is #224 (open): "end rules for open-ended blocks (hr_recovery maxS,
  kj) in block sequencing". ADR-0001 §5 already fixes the loss rule: "`hr_recovery` blocks fall back to their `maxS`
  with a visible notice".
- **Live edits** (DESIGN §17, #240, designed, not implemented) keep `CompiledWorkout.blocks` immutable for the ride.
  Edits live in an overlay (`suppressed`, `extend_s`, `added`). `effective()` and `block_status()` are pure
  functions in `crank-domain`, and the `ride_edit` log rebuilds the overlay (§17.4). Its invariant: the ridden
  sequence minus added reps is a subsequence of the compiled one.
- **ADR-0008** adds `$variables` (arithmetic only in declarations) and pinned `auto` ramps, with names kept in the tree
  (`Num = Lit | Var`). It is accepted and not implemented.
- **#699** (generation): the generator is "rules + settings" producing "a template family written in the DSL (ADR-0008
  variables and pinned ramps make templates a few `$` values)". Providers decide settings, "never trusted unvalidated".
- **#732** (design in progress): lint, completion and "did you mean" come from the Rust parser and compiler, with no
  grammar duplicated in TS.

### 1.2 The two questions are not the same question

The owner's second wish ("either on total reps or a HR threshold") is a decision **during the ride**: how many reps
happen depends on the rider's heart rate that day. A script that runs **before** the ride (Starlark or any
other) sees no heart rate and can only emit a fixed tree. Giving it live HR would put the script in the ride loop, which
every option forbids (§2.1). So the conditional structure must be a declarative node that the compiler bounds and the
engine evaluates, **whatever language writes the tree**. Question 1 is therefore only about authoring and generation,
and it has to be judged on what a script would add there.

### 1.3 How other tools do conditional workouts

| Tool | Conditional end | Bounds | Source |
|---|---|---|---|
| Garmin devices (Edge) | **Repeat Until**: "repeat a 5-mile step for 60 minutes or until your heart rate reaches 160 bpm", set with "Back to Step" | one condition per repeat | Garmin Edge manual, "Repeating Workout Steps" |
| FIT workout file (Garmin FIT SDK) | `repeat_until_hr_greater_than` / `repeat_until_hr_less_than` (field `repeat_hr`), also `repeat_until_steps_cmplt`, time, distance, calories, power. "Only the workout steps with an Intensity type Active are part of the criteria to stop or continue" | one condition per repeat step, no min/max pair | FIT SDK, Workout file type |
| intervals.icu | no HR-conditional step. "Press lap" ends a step on the lap button (Garmin only). The developer put "end step when HR reached" on a to-do list. Not observed as shipped | n/a | intervals.icu forum, "Workout Builder: Duration types" |
| Zwift `.zwo` | `IntervalsT` has `Repeat`, `OnDuration`, `OffDuration`, `OnPower`, `OffPower`: a fixed count. No conditional element observed in the community reference | n/a | h4l zwift-workout-file-reference |
| TrainerRoad | workouts are fixed. Adaptive Training changes **which** workout comes next, not the structure of a workout in progress | n/a | TrainerRoad support, Adaptive Training Overview |
| crank today | `hr<118 2m..4m 40%`: a step-level condition **with both bounds** | min and max duration | DSL.md |

What we borrow: Garmin and FIT have "repeat until HR" at the **repeat** level, and FIT counts the condition only on active
steps. What we add is crank's own rule from `hr<`: **both bounds always written** (FIT has only the condition, and a
lost strap leaves the device to decide). No surveyed tool used a scripting language for workouts. All of them
(`.zwo` XML, FIT, intervals.icu text, ERG/MRC) are declarative data.

## 2. Decision

### 2.1 Language strategy: (A) keep and extend the declarative DSL

Compared on every axis the brief names (evidence in the cells, crate facts checked 2026-11-27):

| Axis | (A) DSL + ADR-0008 | (B) DSL + optional pre-ride script that emits a tree | (C) script replaces the DSL |
|---|---|---|---|
| Round trip with the outline and canonical text | Holds: tree ⇄ `print_dsl`, property-tested. ADR-0008 keeps names in the tree | Holds for the **emitted** tree only. An outline edit of a generated workout cannot flow back into the script (a program is not a function of its output), so the workout **detaches** from its script on the first edit, or the edit is lost on the next run | Breaks: the outline would have to edit code. Printing a tree back into an equivalent program with its loops is not defined, so the outline's one-undo-step edits (#731) have no target |
| Coach and rider adjustments (DESIGN §7.1, §17) | Unchanged: they act on `FlatBlock`s | Unchanged (same tree) | Unchanged only if the script still emits a tree, which makes it (B) without a stored DSL |
| AI rewrites (#699, ADR-0008 §2.12) | The model writes short grammar-constrained text. The content hash covers the selected node. The parser refuses anything else | Model-written **code** would be executed, which is a new attack surface. starlark-rust says it "should in general not be considered secure against truly malicious code" (`set_max_heap_size` doc) | Same as (B), for every workout |
| Analysis (ADR-0009: a known plan) | `compiled_json` is the plan | Same once emitted. The script and its inputs are provenance only | The plan exists only after evaluation, so a stored workout is not a plan until it runs |
| Safety: nothing in the ride loop, termination | Parse and compile are bounded (`NestingTooDeep`, `ExpansionLimit`, `TraceLimit`) | Runs before the ride only. Termination by `Evaluator::set_max_tick_count` (deterministic, counted per loop back edge). The heap limit is "best-effort and should absolutely not be treated as a way to guarantee bounded memory use" (starlark-rust docs). Then the same gates | Same as (B), on every load |
| Errors and #732 tooling | One grammar: typed kinds, UTF-16 spans. #732 builds lint and completion on it | Two languages and two toolchains: starlark's own LSP and linter (`starlark_lsp`) beside #732's, both inside the WKWebView editor | One toolchain, but #732's design (keywords, units, `$vars`, "did you mean") is redone for a general language |
| Storage and migration | ADR-0008's `vars` (default empty), new optional fields below. Old JSON loads unchanged | A new provenance field (script text, args, host-API version). Old data unchanged | Every stored workout and ride snapshot migrates. The frozen-TS parity vectors (`tests/vectors/*.json`) no longer apply |
| Cost | ADR-0008 (designed) plus §2.3 | `starlark` 0.14.3 (2026-11-21, Apache-2.0, on the deny allow-list): 37 non-optional dependencies (among them `rustyline`, `debugserver-types`, `dashmap`, `regex`, `blake3`, `bumpalo`), a 695 KB crate. Its README says it does "_not_ aim for API stability between releases" or "for minimal dependencies". It cannot go into `crank-domain` (pure, three dependencies, ADR-0001 §2), so it needs a new crate, a `deps-edges` change and a host API (the `step()`/`repeat()` builtins) to design, version and test | (B)'s cost plus the migration |

**What a script would add**, stated fairly: loops and arithmetic at generation time, such as a pyramid of `n` steps, a
ladder built from a list, or `if ftp < 200`. Each of these is either:

- **writable in the DSL already:** a pyramid is a handful of lines, and ADR-0008's variables let it be retuned from one
  line; or
- **generator logic, which #699 places in Rust:** the built-in rules are tested Rust that build `Vec<WorkoutNode>`
  directly and pass the same gates. A provider (local model, user endpoint) returns **settings** validated against a
  schema, never code.

So (B)'s distinct value is **user-written generator programs**. Nobody has asked for that, and it costs a second
language, a dependency tree larger than the rest of the domain crate, and a one-way trip out of the editor.

- **Decision:** (A). Starlark (B) and replacement (C) are **rejected** for now. The reasons are in the table, and
  §1.2 shows the owner's conditional structure needs (A)'s declarative node under every option.
- **What would reopen it** (a new ADR, not a deferral): a real need for user-authored generators that #699's settings and
  template families cannot meet, shown with examples. Rhai (`rhai` 1.27.0, MIT OR Apache-2.0, operation limits) would
  then be evaluated against Starlark. Both would emit trees through the gates of §2.6 and never run during a ride.

### 2.2 ADR-0008 under this decision

**Kept as accepted, implemented first, unchanged.** Two links to this ADR:

- The new numeric fields (`min` reps, the trigger's bpm) are `Num`, so `$min..$maxx[hr>=$hrv]` works under ADR-0008's
  rules (a use by name, arithmetic only in declarations). In a count position `$n` reads `$n..$mx`. `x` follows the
  word without a space, as in `3x` today.
- **A pinned end never pins across a ranged repeat's rep boundary.** A ramp inside a ranged set that pins to the next
  rep's first block would, on an early end, lead into the block after the set instead. ADR-0008 §2.13.1 shows that this
  needs re-pinning during the ride, and the frozen-block rule cannot help, because the decision (§2.5) falls at the rep's
  end, after such a ramp has already started. In the spirit of ADR-0008 §2.3 ("no guessing") it is **an error** that
  blocks Ride this: a new `ValidationKind::PinAcrossRangedRep` naming the ramp and the repeat. A pin inside one rep, or
  to a block outside the set, is unaffected.

### 2.3 The ranged repeat: syntax and model

```ebnf
repeat    = ( COUNT | RANGE ) "x" [ "[" option { [","] option } "]" ] "(" workout ")" ;
RANGE     = COUNT ".." COUNT ;                 (* min..max reps, 1 ≤ min < max *)
option    = SIGNED "w" | SIGNED "%" | "!last"
          | "hr>=" INT                         (* until: HR rises to N bpm *)
          | "hr<"  INT ;                       (* until: HR falls below N bpm *)
```

- `RANGE` and exactly one trigger option go together. A range without a trigger, a trigger without a range, a second
  trigger, and `min >= max` are parse errors with spans (§2.11).
- Same spelling as today's `hr<118 2m..4m` (comparison glued to `hr`, `..` for a bounded range). Every word is a parse
  error in today's grammar (`3..10x` is not a `COUNT "x"`, and `hr>=170` is not an option), so **every text that parses
  today parses to the same tree and prints byte-identically**, which is ADR-0008 §2.15's guarantee kept.
- **Canonical printing:** `3..10x[hr>=170, +5w, !last](…)`: the range in the count position, the trigger first, then
  today's options in today's order.
- **Progression** `[+5w]` applies per rep index exactly as today (rep `k` adds `k`·value), so rep `k` is the same block
  whether or not the set ends early.
- **Nesting:** ranged repeats nest in fixed ones (the owner's `3x(…)`) and in each other. Each **instance** (a group in
  §17.1's `GroupId` sense, one per enclosing iteration) has its own trigger state.

Model (serde, `specta`). Existing JSON loads unchanged:

```rust
pub struct RepeatNode {            // as today; `count` is the maximum
    /* id, label, count, children, progression, skip_last_rest */
    #[serde(default)] pub until: Option<Until>,
}
pub struct Until { pub min: Num, pub trigger: Trigger }   // Num from ADR-0008 (f64 until it lands)
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger { HrAtLeast { bpm: Num }, HrBelow { bpm: Num } }
```

The owner's example, with `!last` so the final short rest gives way to the long rest:

```text
"Warm-up" 10m ramp 45-70%
"VO2 HR sets" 3x("Pump" 3..10x[hr>=170, !last](30s 120%, 15s 50%), "Long rest" 4m 40%)
"Cool-down" 8m ramp 55-40%
```

One instance is 3 × 45 s − 15 s + 4 min = 6:00 at least, and 10 × 45 s − 15 s + 4 min = 11:15 at most. All three take
18:00 to 33:45.

### 2.4 Triggers

| Trigger | v1 | Reason |
|---|---|---|
| `hr>=N`, `hr<N` (bpm) | **yes** | The owner's case. HR is the one signal that is not the controlled quantity in ERG. It matches `hr<` today and FIT's `repeat_until_hr_*` |
| `cad<N` (fatigue: "until I can't hold 80 rpm") | owner item 12, recommended **no** for v1 | Meaningful in ERG, where power is held and cadence falls first. But cadence drops at every block change and in ERG's low-cadence release (ADR-0002 §7), so it needs work-block-only evaluation after settle, a 10 s hold, and a design for the release. Worth its own small amendment once HR triggers have been ridden |
| power (`pwr<N`) | **no** | In ERG the trainer holds power, so the condition says nothing about the rider. In resistance or SIM it would be the rider's own failure; revisit with free/SIM sets |
| time in zone ("until 3 min accumulated at ≥ 170 bpm") | **no** for v1 | A useful VO2 prescription, but it is a cumulative accumulator, not a threshold. It fits the same evaluator later (one more `Trigger` variant) without changing §2.5–§2.10 |
| % HR max / % LTHR | **no** (bpm only) | Today's `hr<` is bpm. ADR-0008 variables let a library workout declare `$hrv`. Zone-relative triggers wait for the athlete model's HR references (ADR-0009 Part 1) to be implemented |

**Signal and debounce.** The trigger reads the same 1 Hz HR the record stores (`SecondRow`, ADR-0009 §2.3.1.1: NaN
where no valid sample arrived). It **holds** after **5 consecutive riding seconds** with a valid HR meeting the
comparison. A NaN second or a paused second resets the run. This rejects a single-sample strap artefact (#258 "HR
garbage") without a smoothing filter whose lag would need its own tuning.

**Hysteresis:** none is needed, because a trigger **latches once per instance**. After it has held, nothing un-holds
it, so there is no chatter at the threshold to suppress. Each new instance (the next outer iteration) starts unlatched.

### 2.5 When a set ends, and HR loss

The decision is made at **rep boundaries**, so a rep is never cut in the middle of its work:

- At the end of rep `k` (1-based; "end" = its last child ends or is skipped), the set **continues** unless one of these
  holds: (a) `k = max`; (b) `k ≥ min` and the trigger has latched at any time since the instance began (during reps
  before `min` included); (c) `k ≥ min` and HR is **lost**.
- **`!last` on a ranged set:** the final rep drops its last child, as today. Because the final rep is only known at run
  time, the decision for rep `k` is taken **when its last child would start**: if the set ends at `k`, that child is
  suppressed and the ride goes straight to the block after the set. If the trigger latches **during** that last child
  (and `k ≥ min`), the set ends at once and the child stops there, which is the same rule applied as late as possible.
- **HR lost** means: the engine's HR-loss state (ADR-0001 §5: HR metrics `null`), or no valid HR second in the last
  10 s, at the decision point. HR loss never pauses the ride (ADR-0001 §5), so a set with a lost strap ends after rep
  `max(min, k)`, the bound with less work. The rider sees "HR lost · set ends after this rep" in the target sub-line.
  If HR returns before the decision point, evaluation simply continues; a run in progress was already reset by the NaN
  seconds.
- **Principle, for every conditional end:** a lost signal resolves to the bound with **less work**. For today's
  `hr<` recovery step that is `max` (a longer rest, ADR-0001 §5, already decided). For a ranged repeat it is `min`.
- **Pause:** workout time and the trigger stop together. Paused seconds are not riding seconds, so they never count
  toward the 5 s hold.
- **The rider always wins:** End set, Skip and suppress (§2.9) act at once, whatever the trigger says.

### 2.6 Compilation and the gates

- **Expand to `max`.** The compiler treats a ranged repeat as `count = max` repeats with a known index, so the
  `FlatBlock` list, `path`/`iteration`, progression and phases are exactly today's rules. A block of a rep beyond `min`
  carries an optional field:

  ```rust
  /// Set when the block belongs to a rep the ride may drop: rep `rep` (0-based) of a ranged group with bounds `min..max`.
  #[serde(skip_serializing_if = "Option::is_none")] pub ranged: Option<RangedRep>, // { group_depth, rep, min, max }
  ```

  `CompiledWorkout` gains `ranged_groups: Vec<RangedGroup>` (each instance's `GroupId`, `min`, `max`, trigger, and
  `!last`) and `tss_range: [f64; 2]`.
- **Every gate runs on the maximum**, which is a superset of anything that can be ridden: errors, the health-cap start
  guard (#218), W′bal (`DepletedReserve`/`LowReserve`), the ERG reachability warning (#627), `ExpansionLimit` and
  `TraceLimit`. Because an early end only **removes** whole reps and never creates a target (DESIGN §17.3), the ridden
  sequence is a subsequence of the validated one, which is §17.1's invariant by construction.
- **Validation added:** `TriggerOutsideRange` (the bpm outside the athlete's rest and max HR, the same check as
  `RecoveryTargetOutsideRange`); `PinAcrossRangedRep` (§2.2). A trigger that a fixed-count repeat would make
  pointless (`min >= max`) is a parse error, not a warning.
- **Phases** are classified on the maximum expansion, as `repeat_once` copies phases today (DESIGN §17.1 step 5, #698).
  An early end changes no block's phase.

### 2.7 Execution: engine and overlay

- **A pure evaluator in `crank-domain`** (`workout::until`, no clock, no I/O): a state machine per instance, fed one
  riding second at a time (`hr: f64`, NaN for none). It returns `Latched` / `Running(n)` and, at a decision point, the
  verdict `Continue | End(reason)`. The engine calls it on each closed second (`append_second`, 1 Hz of the 4 Hz tick)
  and at each decision point of §2.5. Replays and the simulator reproduce it exactly.
- **An early end is an overlay edit.** The engine applies `until_met(group, after_rep)`. It suppresses every block of the
  instance's reps after `after_rep`, plus the dropped `!last` child, with a new suppression kind `until_met` beside
  §17.1's `rider` and `set_ended`. It writes one `ride_edit` row (kind `until_met`, **actor `plan`**, input `plan`,
  `param_json` = `{rep, reason, at_s, value}`). The cursor then moves on to the first block after the set, as the
  sequence already says. Consequences:
  - §17's machinery does all the rest: `effective()` drops the blocks, the road greys them, laps close and open as at
    any boundary, crash recovery and #50's resume rebuild the overlay from the log, and Review reads the same fold.
  - `until_met` is not on the undo stack. It is the plan's own decision, like a block ending on time.
  - The `ride_edit` table of §17.4 is not migrated yet (#240 open). Its `kind` and `actor` CHECK lists include
    `until_met` and `plan` from the first migration. If #240's migration lands first, a forward migration adds them.
- **ERG and ADR-0002:** nothing changes in the control law. The transition into the block after the set is an ordinary
  block boundary: the target moves through the eased ERG target (DESIGN §19, #585), compliance and settle restart per
  block (ADR-0002 §7.8), and the soft start is not involved (it belongs to start, resume and reconnect, §7.5). A trainer
  dropout during a ranged set follows ADR-0002 §6 and §7.5 unchanged. The set's trigger watches HR, not the trainer.
- **Order of work this needs:** #240 (overlay engine step) and #224 (open-ended end rules) come first. The `hr<` step
  end of #224 and this evaluator share the HR-validity and loss rules, so #224 adopts §2.4's validity definition.

### 2.8 Timeline, Ready sheet and estimates

- **Profile and road:** reps up to `min` draw as any block. Reps beyond it draw **hatched / faded** with a bracket
  "up to 7 more · until HR ≥ 170". During the ride, after `until_met`, the dropped reps grey like suppressed blocks
  (DESIGN §17.6). The ride's target sub-line shows "rep 4 · 3–10 · HR 164 → 170", and "HR 170 reached · last rep" once
  latched.
- **Length:** the blocks tile at the maximum, so `total_s` is the maximum (the plan as written). `total_range_s` lower
  bound = the length with every ranged instance at `min`, the upper = `total_s`. No expected value is guessed. A
  "usual reps for this workout" estimate needs ridden history and is ADR-0009's (§2.10), shown only when it exists.
- **TSS range:** `tss_range = [TSS(all min), TSS(all max)]`, each from compiling the workout with those fixed counts
  (the same compiler, so each end is exact for its plan). An added rep adds riding seconds of non-negative power, so a
  count in between is expected to fall inside the range. This is not a proof: inserting a rep changes the 30 s NP
  windows that cross its seams. A property test over generated workouts checks it (§4.2). IF and time in zone show at
  the maximum, labelled "at 10 reps".
- **Ready sheet and library:** the length and TSS ranges (for the owner's sets alone, 18:00–33:45). A ranged workout without a heart-rate strap paired shows on the
  Ready sheet: "No HR: ranged sets will ride their minimum (3 reps)". This is not a refusal, since §2.5 already defines
  the ride.

### 2.9 Rider and coach adjustments

- **End set** ends a ranged set at once (reason `rider_end_set`), below `min` too: the rider's choice wins. **Skip**
  moves through blocks as today. A skipped block still completes its rep for counting. **Suppress** of upcoming
  optional reps is allowed (a smaller max).
- **`repeat_once` on a ranged group is refused** with a new code `ranged_repeat`: the condition owns the count, and an
  extra rep beyond `max` would leave the validated maximum (§2.6). Added to DESIGN §17.2's refusal table.
- **Coach:** the ±% and the rider's ± apply to block targets as today (DESIGN §7.1). A coach `replan` may propose
  `end_set` on a ranged set (always an ask, §17.2). The coach never changes `min`, `max` or the trigger.
- **ADR-0008 §2.13** order is unchanged: variables → pins → compile → ride-time adjustments. A trigger value
  bound to `$hrv` is resolved at compile time and is not re-read during the ride.

### 2.10 Analysis (ADR-0009) and FIT

- **Planned vs ridden per instance:** reps ridden, the reason (`condition_met` with the second and HR value at which
  it latched; `max_reached`; `signal_lost`; `rider_end_set`; `skipped`), from the `ride_edit` log and laps. Dropped
  reps are `until_met`, not `skipped`. Completion, outcome and "on target" count only ridden blocks, so a set that
  ended on its condition is **completed as planned**.
- **A new set statistic** (ADR-0009 §2.4): "reps to trigger" and "time to trigger" for rides of the same workout. It is
  shown as a trend with **no verdict**, as decoupling is (ADR-0009 owner decision 13): HR on a given day depends on
  heat, sleep and caffeine, so fewer reps to 170 bpm is not on its own a fitness signal.
- **FIT activity export** (#257) writes laps as ridden, so a ranged set is just its ridden reps; nothing changes.
  A future **FIT workout** export (not planned) maps a ranged repeat to `repeat_until_hr_greater_than` / `_less_than`
  with `count = max` dropped: FIT has no min/max pair (§1.3), so the export would be lossy and must say so. A FIT
  workout import of `repeat_until_*` must ask for bounds, since this ADR refuses an unbounded set.

### 2.11 Errors, #732 and #699

- New `DslErrorKind`s, each with a span: `RangeWithoutTrigger` (at the range), `TriggerWithoutRange` (at the
  trigger), `DuplicateTrigger` (the second trigger), `InvalidRepRange { min, max }` (at the range). Examples:

  | text | message | highlighted |
  | --- | --- | --- |
  | `3..10x(30s 120%, 15s 50%)` | A rep range needs an end condition, e.g. 3..10x[hr>=170](…) | `3..10x` |
  | `10x[hr>=170](30s 120%)` | 'hr>=170' needs a rep range, e.g. 3..10x[hr>=170](…) | `hr>=170` |
  | `5..5x[hr>=170](30s 120%)` | A rep range needs min below max (5..5); write 5x(…) for a fixed count | `5..5x` |

- **#732** gains vocabulary, not a new tool: `..x` after a count, `hr>=`/`hr<` in repeat options, and "did you mean"
  for `hr=>170` and `hr>170` (suggest `hr>=170`, with "triggers are `hr>=N` or `hr<N`").
- **#699** may use ranged sets in template families. A provider still returns settings, never text that skips the
  gates.

## 3. Alternatives considered

- **(B) Starlark generator layer, (C) script replaces the DSL:** §2.1. Rejected on round trip, attack surface for AI
  output, a second toolchain beside #732, and cost. Under both, the owner's conditional still needs §2.3.
- **A `while`/`until` construct with no bounds** (FIT's shape): rejected. A lost strap would leave the length undefined,
  the gates could not run on a known maximum, and the Ready sheet could not state a length.
- **A step-level "ride until HR ≥ 170" (like `hr<`) instead of a repeat:** it cannot express "intervals of fixed time
  … until": the condition spans several reps of work and rest.
- **Compile at `min`, append reps at run time** (like `repeat_once`): the gates would validate less than may be ridden.
  It breaks §17.1's subsequence invariant and §17.3's "an edit cannot create a target". Expanding to `max` and
  suppressing keeps both.
- **Ending the set mid-rep when the trigger holds:** cuts a work interval at an arbitrary second, and the rep count
  stops meaning anything for analysis. Only `!last`'s final rest is cut, because cutting it is the purpose of `!last`.
- **HR loss falls back to `max`** (as `hr<` does): for a work set that is the bound with **more** work, exactly when the
  app has lost its view of the rider's effort. Rejected by §2.5's principle.
- **Smoothing HR (moving average) instead of a 5 s hold:** adds lag and a tuning constant. The hold rejects the same
  artefacts and is exact to test.
- **An expected length from a model now:** no data yet. A guess would show as fact on the Ready sheet. A range is
  honest, and ADR-0009's history can add an estimate later.

## 4. Consequences

- The owner's structure is writable in one line and rides safely: bounded, validated at its worst case, ending on HR.
  It falls back to less work if the strap drops.
- **No new language, no new dependency.** ADR-0008 is implemented as accepted.
- The domain contract changes: `RepeatNode.until`, `FlatBlock.ranged`, `CompiledWorkout.{ranged_groups, tss_range}`, new
  `DslErrorKind`s and `ValidationKind`s. The TS bindings are regenerated. Stored workouts and snapshots deserialise
  unchanged.
- DESIGN §17 gains a suppression kind (`until_met`), an actor (`plan`) and a refusal (`ranged_repeat`). The amendment
  text lands with the engine step.
- Hard: a script user cannot write a generator program. §2.1 names the evidence that would reopen it.

### 4.1 Implementation issues, in order (filed on acceptance)

1. ADR-0008 implementation (#694), unchanged. Prerequisite for `Num` in the new fields (or `f64` first, `Num` with
   ADR-0008).
2. #240 engine step (overlay) and #224 (end rules), with #224 adopting §2.4's HR validity and §2.5's loss principle.
3. Domain: grammar, model, canonical printing, errors, compile at max, `ranged_groups`, `tss_range`, the two new
   validations, the `until` evaluator.
4. Engine and core: decision points, `until_met` edits and their log rows, the `ranged_repeat` refusal, status fields
   for the sub-line.
5. App: hatched optional reps, the sub-line, Ready-sheet ranges and the no-HR note. Design-harness fixtures.
6. Analysis (ADR-0009): per-instance outcome, the "reps to trigger" set statistic.
7. #732 vocabulary and "did you mean" entries; DSL.md grammar and examples.

### 4.2 Test plan (for the implementation issues)

- **Round trip (properties):** the generators of DSL.md and ADR-0008 extended with ranged repeats (nested, with
  progression and `!last`, `Num` bounds). `parse(print(t)) == t`, printing idempotent. Every existing golden text prints
  byte-identically.
- **Compile:** `compile(ranged)` equals `compile` with `count = max` in every field except the new ones. `tss_range`
  equals the TSS of the all-min and all-max fixed-count compiles. `total_range_s[0]` equals the all-min length. For
  generated workouts, the TSS of every intermediate count lies inside `tss_range` (§2.8). A counterexample reopens §2.8
  before release.
- **Evaluator (properties and goldens):** latch only after 5 consecutive valid seconds; a NaN or paused second resets.
  Latched before `min` → ends at `min`. Never past `max`. HR lost at a decision point past `min` → ends there. `!last`:
  the dropped child is never started when the decision is taken at its start, and stops at once when the latch falls
  inside it. The verdict depends only on the input seconds (replay equality).
- **Overlay:** for generated ranged workouts and HR traces, the ridden sequence is a subsequence of the compiled one,
  ends of reps only (never a partial rep, except `!last`'s child), and `effective` after `until_met` equals compiling
  with the ridden counts, phases aside (#698).
- **Engine scenarios** (simulator, `VirtualClock`): the owner's example with a `SimHeart` that crosses 170 at rep 2, 5
  and never. A strap loss in rep 4. A loss during rep 2 that returns. Pause inside the 5 s hold. End set below `min`.
  `repeat_once` refused `ranged_repeat`.
- **Errors:** each row of §2.11 (kind, message, span), and `PinAcrossRangedRep` / `TriggerOutsideRange` (kind,
  severity, node).

## 5. Open questions

None blocking. The owner items above are the questions. Anything the review adds is filed and linked here before
acceptance.

| Question | Issue | Milestone |
|---|---|---|

## 6. References

Checked 2026-11-27.

- starlark-rust README (Meta): https://github.com/facebook/starlark-rust (determinism, "do _not_ aim for API stability",
  "do _not_ aim for minimal dependencies"; recursion as an extension). Last push 2026-11-25, not archived.
- starlark-rust `Evaluator` (`starlark/src/eval/runtime/evaluator.rs`, main at blob `b8f6d6ac`): `set_max_tick_count`,
  `set_max_heap_size` (its doc: "should in general not be considered secure against truly malicious code", and
  "best-effort"), `set_max_callstack_size`, `set_check_cancelled`.
- crates.io API: `starlark` 0.14.3 (2026-11-21, Apache-2.0, 695 202 bytes, 37 non-optional dependencies); `rhai`
  1.27.0 (2026-10-21, MIT OR Apache-2.0); `mlua` 0.12.3 (MIT, a C Lua runtime, not considered).
- Starlark language spec: https://github.com/bazelbuild/starlark/blob/master/spec.md
- Garmin Edge manual, Repeating Workout Steps (Repeat Until):
  https://www8.garmin.com/manuals/webhelp/GUID-D9E9CF32-5A89-4140-8B6A-0A61633E397F/EN-US/GUID-138B7254-37CB-49E6-A9D2-CA3F8BAEC4CE.html
- Garmin FIT SDK, Workout file type (`repeat_until_hr_greater_than`, `repeat_hr`, active-steps rule):
  https://developer.garmin.com/fit/articles/file-types/workout.html
- intervals.icu forum, Workout Builder: Duration types: https://forum.intervals.icu/t/workout-builder-duration-types/69924
- intervals.icu forum, Workout builder: https://forum.intervals.icu/t/workout-builder/1163
- Zwift workout file reference (community): https://www.github.com/h4l/zwift-workout-file-reference/blob/master/descriptions.yaml
- TrainerRoad, Adaptive Training Overview: https://support.trainerroad.com/hc/en-us/articles/4404060687387-Adaptive-Training-Overview
- In repo: `docs/DSL.md`; `crates/crank-domain/src/workout/{model,dsl,compile,compiled}.rs`;
  `crates/crank-engine/src/engine.rs` (block sequencer); ADR-0001 §2, §5; ADR-0002 §6, §7; ADR-0008; ADR-0009 §2.3,
  §2.4; DESIGN §7.1, §17, §19; `deny.toml`.
