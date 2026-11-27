# ADR-0011: Workout text editor: diagnostics with fixes, "did you mean", and completion from the core

- **Status:** Proposed (for owner review); not implemented
- **Date:** 2026-11-27
- **Issue:** #732
- **Related:** DESIGN §5.7 (editor, undo groups, outline edits #653), §5.9 (desk keys), `docs/DSL.md` (grammar, errors),
  ADR-0008 (`auto`, `$variables`, its errors §2.3 / §2.14), ADR-0010 (ranged repeats `3..10x[hr>=170]`, its errors
  §2.2 / §2.6 / §2.11; its owner item 12 assigns their editor support here), ADR-0001 §3–4 (domain contract, bindings), #627 (slew
  warnings), #111 (editor), #110 (save)

## For the owner

Nothing is implemented. Each line is a decision that needs a yes or a no; the section in brackets has the reasons and
the rejected alternatives.

1. **Replace the text box with CodeMirror 6** (`@codemirror/view`, `state`, `lint`, `autocomplete`; MIT), not a
   hand-rolled overlay on the `<textarea>` and not Monaco. It adds about 310 KB minified / 101 KB gzip to the app
   (measured, §2.1), about 1.6 × React and ReactDOM.
2. **One undo history: the editor model's.** CodeMirror runs without its own history; Cmd/Ctrl+Z keeps undoing the whole
   workout (`GROUP_MS` 500 ms bursts, `HISTORY_LIMIT` 100, outline edits one step each), and the text box is a
   controlled view of the model's draft (§2.2).
3. **The core is the only source of diagnostics, suggestions, fixes and completions.** `parse_dsl`'s answer gains fix
   edits and node spans, and a new command `dsl_complete(text, cursor)` answers completions; TypeScript holds no
   grammar, no keyword list and no matching (§2.3, §2.6). This covers ADR-0008's and ADR-0010's vocabulary and errors
   in the same tables, not in a second tool (§2.5, §2.6).
4. **The first parse error only**, as today: the parser stays fail-fast, with no error recovery (§2.4).
5. **"Did you mean" uses `strsim` 0.11.1 `osa_distance`** (adjacent swaps count as one edit), compared case-insensitively
   against only the words valid in that position, with rustc's threshold (at most ⌊max(len, 3) / 3⌋ edits, so one edit
   for words of 3 to 5 letters and two for 6 to 8), at most 3 suggestions, all at the smallest distance (§2.5);
   ADR-0010's `hr>=` is matched the same way (`hr=>170`, `hr>170` → `hr>=170`).
6. **A fix is offered only when the core has checked that the whole fixed text parses.** A fix for the first error
   while a later error remains is not offered (the rider sees the message; the fix appears once the rest parses), so
   every offered fix commits a tree. Ambiguous errors (`10 50%`: seconds, minutes, kJ or a repeat?; a rep range with no trigger: which
   bpm?) get no fix, only the message. Compile errors (ADR-0008's pins, ADR-0010's `PinAcrossRangedRep`) get none
   either: their example value is a hint in the message, not the rider's intent (§2.5).
7. **Completion comes from the core by cursor context** (§2.6 table), `hr>=` offered only in a ranged repeat's options
   and only once. It opens while typing a word and on Ctrl+Space;
   **Enter accepts, Escape closes, Tab never accepts** (Tab keeps moving focus out of the text box) (§2.7).
8. **Lint beyond errors:** compile warnings and errors are underlined on their block (#627's slew warning, once decided,
   and ADR-0010's `PinAcrossRangedRep`, `TriggerNotWatched`, `TriggerOutsideRange` come the same way);
   ADR-0008's unused variable is a **hint** that never blocks saving, with a "Remove" fix; **no style lint** and
   no "format" fix, because canonical printing drops comments (§2.8).
9. **A fix is exactly one undo step, committed at once:** the core already parsed the fixed text to offer the fix
   (item 6), so the fix carries that tree and its spans and the editor commits them synchronously as a step of their
   own, ending the typing burst. No parse round-trip, so no wait for Save and no race with the next keystroke, which
   starts a new burst. Accepting a completion is typing and joins the burst (§2.2).
10. **Screen readers hear a diagnostic once, when the burst ends** (500 ms pause, the same `GROUP_MS`), through a polite
    live region, not on every keystroke; the message stays the text box's description, with the fix as a button under
    it (§2.7).

Everything else (spans, wording, refusals, tests) follows from these.

## 1. Context

The owner asked (#732, 2026-11-27) for a linter and autocomplete in the editor's text view, and for errors that help:
"did you mean …?" by fuzzy matching, with a one-click fix.

What exists (checked on `dev` at 0438c9b1):

- **The text view is a `<textarea>`** (`app/src/editor/WorkoutEditor.tsx`, `id="editor-dsl"`). Every change calls
  `commands.parseDsl` and feeds the answer to the pure model (`editor/model.ts` `typed`, `parsed`). Because a textarea
  cannot style a range of its own text, a problem is shown under it: ⚠, the message, line and column, and the line with
  the span marked (`problemLine`). Cmd/Ctrl+Z is caught on the editor root and goes to the model's `undo` / `redo`
  (DESIGN §5.9's one exception).
- **The parser is fail-fast.** `parse_dsl(text, ftp, ids)` (`crates/crank-domain/src/workout/dsl.rs`) returns
  `Result<Vec<WorkoutNode>, DslError>`; `DslError { kind: DslErrorKind, message, offset, length }` (`model.rs`) has a
  UTF-16 span. The core's facade (`crank-core/src/core.rs` `Core::parse_dsl`) answers
  `DslParse::Parsed { nodes } | Invalid { problem: DslProblem }`, and `DslProblem { message, offset, length }`
  (`crank-core/src/dto.rs`) drops the kind ("nothing in the UI branches on it").
- **Compile issues have no span.** `compile_workout` returns `ValidationIssue { kind: ValidationKind, severity:
  Severity (Error | Warning), node_id: Option<String>, message }` (`compiled.rs`); the editor shows them in the Summary.
  Error-severity issues block Save and Ride this (DESIGN §5.7).
- **Keywords are string literals inside the parser** (`"ramp"`, `"lap"`, `"free"`, `"!last"`, `"!lower"`, `"!cap"`,
  `"!coach"`, the units `s m h kj % w bpm rpm`), matched case-insensitively. There is no edit distance anywhere in
  `crank-domain` or `app/src/editor`.
- **ADR-0008 (accepted, not implemented)** adds `auto` (a ramp end only), `$name` variables declared before the first
  block, and their errors with spans (§2.14), several of which already name the fix in their message
  ("Put spaces around '=': $thr = 95%").
- **ADR-0010 (accepted by the owner 2026-11-27, PR #737; not implemented)** adds the ranged repeat
  `3..10x[hr>=170, !last](…)` (with variables `$lo..10x`, `3..$hi x`, `$lo..$hi x[hr>=$hrv]`: `x` glued after a
  number, spaced after a variable), the parse errors `RangeWithoutTrigger`, `TriggerWithoutRange`, `DuplicateTrigger`,
  `InvalidRepRange` (its §2.11), and the compile errors `PinAcrossRangedRep`, `TriggerNotWatched`,
  `TriggerOutsideRange` (§2.2, §2.6). Its owner item 12: "#732 gains vocabulary (`..x`, `hr>=` in repeat options) and
  the new errors, not a second tool".
- **Mocks answer from the core's own answers** (`app/src/views/editorFixture.ts`, `workoutsBackend.ts`, from
  `crates/crank-core/tests/fixtures/editor.json`), never from TypeScript logic.

## 2. Decision

### 2.1 Editor component: CodeMirror 6

**Chosen:** CodeMirror 6, the packages `@codemirror/view` 6.44.2, `@codemirror/state` 6.7.6, `@codemirror/lint` 6.9.8
and `@codemirror/autocomplete` 6.20.4 (versions of 2026-11-27; all MIT, as are their dependencies `@lezer/common`,
`@lezer/highlight`, `@lezer/lr`, `@codemirror/language`, `style-mod`, `w3c-keyname`, `crelt`,
`@marijn/find-cluster-break`, so `scripts/check-npm-prod-licenses.py`'s allowlist covers them). Not used:
`@codemirror/commands`' history (§2.2), any CodeMirror language mode (there is no grammar in TS, §2.3; plain text
with the mono face).

What it gives that the textarea cannot, each a tested library primitive (owner rule 2026-11-14):

- marks on a range of the text (lint underlines and the AI diff's struck-through / highlighted text, DESIGN §5.7) as
  decorations, instead of a copy of the line under the box;
- diagnostics with actions (`Diagnostic.actions`: the fix buttons), set from outside the view (`setDiagnostics`, so
  the model, not a lint source, decides when they change), a tooltip on hover, `F8` to the next diagnostic and
  `Mod-Shift-m` for the diagnostics panel (`lintKeymap`);
- a completion popup with an async source and abort on change, ARIA combobox semantics (`aria-autocomplete="list"`,
  `aria-activedescendant` on a `role="listbox"`), and a polite announcer (`EditorView.announce`);
- the content is `role="textbox"` with `aria-multiline="true"`; labelled by the Text heading (`aria-labelledby`
  through `EditorView.contentAttributes`, since a `<label for>` cannot name a `div`), so
  `getByRole("textbox", { name: "Text" })` keeps finding it.

**Bundle, measured** (esbuild, minified ESM, `gzip -9`, 2026-11-27): view + state 205 KB / 66 KB; with lint and
autocomplete 310 KB / 101 KB. For scale, React 19.2 + ReactDOM client is 193 KB / 60 KB. The app ships from disk inside
the Tauri bundle, so this is load and memory, not network; the implementation PR reports the real `vite build` delta.

**Costs, stated:**

- **Tests in jsdom.** `fireEvent.change(textbox, …)` (today's `WorkoutEditor.test.tsx`, `save.test.tsx`) does not drive
  a contenteditable. The editor gets one test seam, `setText(view, text)` (a CodeMirror transaction, the same path a
  keystroke takes after the DOM), and real typing, IME and dead keys are covered in the design harness (Playwright,
  Chromium and WebKit: `just design`, `just design-webkit`) and the real-app e2e.
- **WKWebView.** WebKit is a supported CodeMirror engine, but Playwright's WebKit is not Tauri's WKWebView. The
  implementation PR's `just design-webkit` scenes cover typing, a dead-key composition in a label (`"Côte"`), the
  completion popup and a fix; one owner check in the built macOS app (typing, Cmd+Z from the keyboard **and** from
  Edit ▸ Undo, dictation) is batched with that PR, since the native menu path can reach the contenteditable's own undo.
- **Selectors.** `app/e2e/specs/mvp-selectors.mjs` `editorText` (`textarea#editor-dsl`) and the design scenes'
  `fills` change to the CodeMirror content; the layout audit's invariants apply unchanged (#630).

**Rejected:**

- **Keep the textarea, add a mirrored `<pre>` behind it for underlines and a hand-made popup.** Every part is a
  primitive the owner rule says not to hand-roll: caret coordinates for the popup, scroll and wrap sync between two
  elements, a combobox with active-descendant, IME. Each has known edge cases CodeMirror already tests.
- **Monaco.** Several MB with web workers and its own loader setup in Vite, built for a desktop IDE; far more than a
  one-pane DSL needs.
- **Ace.** The same features (markers, completion) on an older input model (a hidden textarea under a rendered
  layer); no advantage over CodeMirror for this use.

### 2.2 One history, a controlled view

- CodeMirror runs **without** `history()` and binds no undo key. The editor root's existing Cmd/Ctrl+Z listener stays
  the only undo, so tree, text and outline keep one history (DESIGN §5.7: `GROUP_MS` 500, `HISTORY_LIMIT` 100, an
  outline edit one step). The text component also handles `beforeinput` with `inputType` `historyUndo` /
  `historyRedo` (how WebKit delivers Edit ▸ Undo and the menu's Cmd+Z to a contenteditable): it calls
  `preventDefault()` and routes them to the model's `undo` / `redo`, so the DOM's own undo never runs; the §2.1 owner
  check confirms the menu path.
- **The model's `draft` is the truth; the view follows it.** A change the rider makes in CodeMirror (a keystroke, an
  accepted completion) calls `edit(text, at)` as today. A change that comes from the model (undo, redo, an outline
  edit, **a fix**, Accept of an AI rewrite) is made in the model first (`undo`, `applied`, `fixed`, …) and then
  dispatched into the view, annotated as external (a CodeMirror `Annotation`), and the view's update listener never
  calls `edit` for an external transaction, so following the model never starts a typing burst. **A fix is never
  dispatched into the view by itself:** its button and CodeMirror's diagnostic action both call `fixed` only, and the
  view follows the new draft like any model change. The fix's `edits` are used by the core to check it and by the view
  only to mark which range the fix replaces. The dispatch is one replacement of the changed region: the common prefix and suffix of the
  old and new text are kept, so the cursor stays put when it lies outside the change. That trim is a few lines with a
  property test (old text with the region replaced equals the new text); a diff library would only improve where the
  cursor lands after an edit made elsewhere, so it is not worth a dependency (`@codemirror/merge`'s `diff` was
  considered).
- **When the view is not touched:** nothing is dispatched when the model's `draft` equals the view's document (the
  change came from the view itself), and nothing while `view.composing` (an IME or dead-key composition is in
  progress): an external change that arrives then is applied at `compositionend`, after the composed text has reached
  the model, so a composition is never cut.
- **Undo steps:** accepting a completion is typing (it joins the burst like the keys it replaces). **A fix is a step of
  its own, committed synchronously**, through one new model function, `fixed(s, base, fix)`. The core offers a fix only
  after parsing the whole fixed text (§2.5), with the core's own id source, so the fix carries that parse's result:
  `DslFix.result = { text, nodes, spans, hints }`. `fixed` applies only if `s.draft === base`, the draft the fix was
  offered for (a fix from an older answer is refused: its edits are in that text's coordinates). The rider never sees
  a stale fix: when the draft moves on, the marks and fix buttons of the previous answer are removed at once, and the
  next answer brings its own, so the refusal is only the model's guard for a click racing a keystroke, a no-op with
  nothing shown. Then it commits
  `fix.result` with today's `commit(…, grouped = false)`, the selection following its path as `parsed` does, and
  sets `typing: null`. There is no parse round-trip: the saved tree is the fixed one at once, so Save and the outline
  never wait for it, and nothing is in flight for a keystroke to make stale. The next keystroke finds `typing: null`,
  so it starts a new burst whose first commit is a new step (`grouped = false`), and later keys join that burst, not
  the fix. So Cmd+Z after a fix undoes exactly the fix, and history keeps holding parsed states only (`model.ts`: "History holds saved states only"). **Rejected:** history
  steps for unparsed text (a fix whose result still has an error): undo would restore drafts with no tree, which the
  outline, Save and the compiled view cannot show, a larger change to the model than #732 needs.
- **The AI rewrite lock** (#112, `model.ts` `lock`): the view becomes read-only (`EditorState.readOnly` and
  `EditorView.editable` in a compartment), fix buttons and completion are disabled, selection and copy still work.
  The model already refuses every edit while locked; this keeps the view from showing keys the model drops.

### 2.3 The diagnostics contract

Diagnostics stay the core's: spans, severity and message come from Rust; TypeScript only places them.

**Domain** (`crank-domain`, new module `workout/lint.rs`, no new public behaviour of `parse_dsl`):

```rust
/// A replacement of a UTF-16 span of the text (length 0 = insertion). Offsets are in the text as parsed (before
/// any edit of the fix is applied).
pub struct DslEdit { pub offset: usize, pub length: usize, pub insert: String }
/// A fix the core has checked (§2.5): what the UI shows after "Did you mean", and its edits.
/// `edits` are sorted by offset and never overlap or touch, all in the original text's coordinates: exactly
/// CodeMirror's `ChangeSpec[]` semantics. The core applies them last-to-first when it checks the fix, and
/// `result.text` is exactly the text they produce; the view never applies them itself (it shows their range, and
/// accepting the fix commits `result` through the model, §2.2).
/// `result` is the parse of the fixed text that made the fix offerable (§2.5), so the editor commits it without
/// parsing again (§2.2).
pub struct DslFix { pub label: String, pub edits: Vec<DslEdit>, pub result: FixedText }
pub struct FixedText { pub text: String, pub nodes: Vec<WorkoutNode>, pub spans: Vec<NodeSpan>, pub hints: Vec<DslHint> }
/// The UTF-16 span of each node's head, in pre-order of the parsed nodes: a step from its end token to its last
/// attribute (its label excluded); a repeat its count and options (`3x[+5w]`, `$n x`,
/// ADR-0010's `3..10x[hr>=170, !last]` and `$lo..$hi x[hr>=$hrv]`), not its body.
pub struct NodeSpan { pub offset: usize, pub length: usize }
pub enum DslHintKind { UnusedVariable }                          // ADR-0008 §2.6; the only hint (§2.8)
pub struct DslHint { pub kind: DslHintKind, pub message: String, pub offset: usize, pub length: usize, pub fixes: Vec<DslFix> }

pub fn parse_dsl_report(text: &str, ftp: f64, ids: &mut impl FnMut() -> String) -> DslReport;
pub enum DslReport {
    Parsed { nodes: Vec<WorkoutNode>, spans: Vec<NodeSpan>, hints: Vec<DslHint> },
    Invalid { error: DslError, fixes: Vec<DslFix> },
}
```

`parse_dsl` keeps its signature (the frozen TS parity tests and the round-trip properties of #68 do not change); the
parser records each node's head span while it parses, and `parse_dsl` is `parse_dsl_report` without the extras.

**Boundary** (`crank-core/src/dto.rs`, offsets checked into `u32` as `DslProblem` does today):

```ts
type DslParse =
  | { outcome: "parsed"; nodes: WorkoutNode[]; spans: NodeSpan[]; hints: DslHint[] }
  | { outcome: "invalid"; problem: DslProblem };          // DslProblem gains `fixes: DslFix[]`
// DslFix = { label, edits: DslEdit[], result: { text, nodes, spans, hints } }; WorkoutDetail gains `spans: NodeSpan[]`
```

**Compile issues get their place by node:** the editor already compiles the committed tree; an issue's `node_id` is
found in that tree in pre-order, and the span at that index is the one underlined. The spans are kept beside the tree
in the model's snapshot (`saved: { nodes, text, spans }`, and so in `past` and `future`), so undo restores them with
the text and no re-parse is needed.

**Who produces the spans, for every way a tree is committed:**

| Commit | Spans from | Extra cost |
|---|---|---|
| typing (`parsed`) | that parse answer | none |
| a fix (`fixed`) | the fix's `result`, from the check parse (§2.5) | no extra parse (that parse was needed to offer the fix); the answer grows by up to 3 trees, only while the text does not parse, and only for texts up to 64 KiB (§2.6 "Cost per keystroke") |
| an outline edit (`applied`, #653/#731) | the read-back parse of the printed text: `outlineEdit` already prints (`print_dsl`) and parses the result (`parse_dsl`) and commits **the tree the core read back** (DESIGN §5.7), so `applied` takes that answer's `spans` too | none |
| opening the editor (`load`) | `WorkoutDetail.spans`: the core's `workout_get` already returns the canonical text (`dsl`) beside the tree, and now also the spans of parsing that text; a new workout's empty text has none (`[]`) | one parse per `workout_get` in the core; spans are by pre-order index and `parse(print(t)) == t` structurally (docs/DSL.md), so they match the stored tree although that parse's ids are discarded. The editor opens with its spans in hand and never waits |
| Accept of an AI rewrite (#112, not built yet) | the parse of the accepted text: #112 commits through `parsed` or `applied` like every other change, so it takes the answer's spans | none |
| undo, redo | the snapshot | none |

`load` (from `EditorStart.spans`, filled from `WorkoutDetail.spans`) and `applied` gain a required `spans` argument;
no state of the model, the opening one included, is without spans.

Issues without a node (`EmptyWorkout`, `MissingCooldown`, `InvalidFtp`) stay in the Summary only. While the draft does
not parse, only the parse error is drawn (the compile issues describe the saved tree, whose spans no longer match the
text); the Summary keeps them as today.

- **Rejected:** `print_dsl` returning spans: a second span producer in the printer, to keep equal to the parser's, for
  a case (outline edits) that already parses its printed text. **Rejected:** `load` with optional spans filled by a
  later parse: a state without spans, and an answer to match against the snapshot it belongs to, for a parse the core
  can do inside `workout_get`.

**Severity in the view:** a parse error and a compile `Error` are errors (Save and Ride this disabled, as today); a
compile `Warning` is a warning (saves); a lint hint (§2.8) is a hint, never in the Summary, never blocking.

- **Rejected:** a grammar or keyword list exported to TS for the UI to match against: two places to keep in step, the
  very drift #732 asks to avoid. **Rejected:** a `dsl_check` command that parses and compiles in one answer: the
  editor compiles only committed trees, and two compiles of one tree per keystroke would race the existing one.
  **Rejected:** spans keyed by node id: ids are fresh on every parse, so a restored snapshot's ids would not match.

### 2.4 The first error only

The parser stays fail-fast, so a draft has at most one parse error, the first in the text, as today. Error recovery
(resynchronising at the next separator to report later errors) would touch every parse path, put the TS-parity and
round-trip properties at risk for text that is not a workout, and still guess where a broken repeat ends. A rider
fixes one error at a time, and the first is the one whose fix can be checked (§2.5).

### 2.5 Suggestions and fix edits

**Vocabulary, one table per position, used by the parser itself:** the keyword and unit strings become `const` tables in
`dsl.rs` (step ends: `lap`, `hr<`; targets and attributes: `ramp`, `free`, `!lower`, `!cap`, `!coach`; units after a
number: ends `s m h kj`, targets `% w bpm`, cadence `rpm`; repeat options `!last`; ADR-0008: `auto` as a ramp end,
`$names` declared above the position with a type that fits it; ADR-0010: the rep range `..` with its `x`, and the
trigger `hr>=` among the repeat options). The parser matches against these tables, and the
suggester and completion (§2.6) read the same ones, so a keyword cannot exist in one and not the others.

**Matching:** `strsim::osa_distance` (optimal string alignment: insert, delete, substitute, swap two neighbours; the
algorithm rustc's `edit_distance` uses), on the lower-cased word against each candidate valid where the error is.
A word that starts with a digit or a sign is a number with a unit (below); a word of the form letters, comparison,
number (`hr=>170`) is matched by its part before the number (`hr=>` against `hr>=`, `hr<`); any other word is matched
against the keywords. A candidate is suggested when its distance is at most ⌊max(n, 3) / 3⌋, n the typed word's length in characters
(rustc `find_best_match_for_name`'s default), and no other candidate is closer; up to 3 suggestions at the smallest
distance, in table order. For a number with a unit (`95%%`, `4mm`, `90rmp`), only the unit part is matched, against
that position's units, and only when it is at least 2 characters long: a one-letter unit (`5q`) is one edit from every
one-letter unit, which is a guess, not a suggestion. A variable (ADR-0008, `$thrr`) is matched by its name against the
variables declared above that position with the right type, never against keywords.

- `strsim` 0.11.1 (rapidfuzz/strsim-rs, MIT, no dependencies, MSRV 1.56) is already in `Cargo.lock` (through
  `darling_core`, a build-time dependency), so `crank-domain` adds a direct dependency and the lock gains nothing.
- **Rejected:** Jaro-Winkler (clap's choice): a similarity score with a prefix bonus, whose cut-off would have to be
  tuned per word length for 3–5 letter keywords; a count of edits is easier to bound and to explain.
  **Rejected:** plain Levenshtein: `rmap` → `ramp` would cost two. **Rejected:** a hand-written distance: the owner
  rule, and `strsim` is the de facto crate.

**Which errors get a fix** (each fix is re-checked, below):

| Error (`DslErrorKind`, or ADR-0008 §2.14 row) | Fix | Example |
|---|---|---|
| `UnknownToken` (an attribute word) | the near keyword or unit | `rmap 50-75%` → `ramp`; `fre` → `free`; `!cpa` → `!cap`; `90rmp` → `90rpm`; `140bmp` → `140bpm` |
| `InvalidStepEnd` | the near end keyword or unit | `lpa` → `lap`; `4mm` → `4m`; `10kjj` → `10kj` |
| `UnknownRepeatOption` | `!last`, or the near unit | `[!lst]` → `!last`; `[+5ww]` → `+5w` |
| `ReversedPowerRange` | two fixes: the ramp the message names, or the range swapped | `75-50%` → `ramp 75-50%` / `50-75%` |
| `MissingUnit`, `MissingPowerTarget`, `UnclosedRepeat`, `MissingSeparator`, `DuplicatePowerTarget`, `NestingTooDeep`, numeric bounds | none: the intent is not recoverable from the text | `10 50%` keeps today's message listing the units |
| ADR-0008: unknown variable | the near declared variable | `4m $thrr` → `$thr` |
| ADR-0008: `auto` misspelt as a ramp end | `auto` | `ramp aut-75%` → `ramp auto-75%` |
| ADR-0010: a misspelt trigger in repeat options (`UnknownRepeatOption`) | `hr>=` with the number kept | `3..10x[hr=>170]`, `[hr>170]` → `hr>=170` |
| ADR-0010: `hr<N` in repeat options | none: its own message ("a set ends on `hr>=N`; for a recovery, use the step end `hr<N min..max`"), since the rider meant something else | `3..10x[hr<120]` |
| ADR-0010: `InvalidRepRange` with `min = max` | the fixed count its message names, with the trigger removed (two edits, checked as one fix) | `5..5x[hr>=170](…)` → `5x(…)` |
| ADR-0010: `RangeWithoutTrigger`, `TriggerWithoutRange`, `DuplicateTrigger`, `min > max` | none: which bpm, which minimum, which trigger or which bound is meant is not in the text | `3..10x(…)`, `10x[hr>=170](…)` |
| ADR-0010: a variable upper end glued to `x` (an unknown variable `$hix`) | the spaced form, when `$hi` is declared | `$lo..$hix[hr>=170]` → `$lo..$hi x[hr>=170]` |
| ADR-0008: the spelling refusals whose message shows the fix | that spelling | `$thr=95%` → `$thr = 95%`; `ramp 45-auto` → `ramp 45%-auto`; `ramp 45%-75%` → `ramp 45-75%`; `$a = $b-30s` → `$b - 30s` |

The core's `message` stays as it is (the TS-parity text, docs/DSL.md "Errors"); suggestions are not appended to it. The
UI writes "Did you mean ramp?" from the fix's `label`, and the fix's button reads "Change to ramp".

**A fix is checked before it is offered:** the core applies each candidate fix's edits to the text (last to first,
so every offset stays in the original text's coordinates) and parses the result with the core's own id source (the same as `parse_dsl`'s, so the ids are fresh and the tree
can be committed as it is, §2.2); the fix
is offered only if **the whole fixed text parses**. A candidate that fails the check is dropped, not shown with a
warning. That costs at most 3 extra parses, only when the text does not parse. **Cost, stated:** with a second error
later in the text, the first error gets no fix until the later one is fixed; the message still names the problem. It
buys the guarantee of §2.2 that every fix commits exactly one undo step. **Rejected:** also offering a fix that only
moves the first error later: its result is a draft with no tree, which cannot be an undo step (§2.2).

### 2.6 Completion from the core

**Command:** `dsl_complete(text, cursor) -> DslCompletion { from: u32, to: u32, options: DslOption[] }` with
`DslOption { label: string, insert: string, detail: string | null }` (`detail`: what the option is, "ramp between two
powers", "% FTP variable · 95 %"), UTF-16 offsets. Refusal: `InvalidInput` when `cursor` is past the end of the text
or inside a surrogate pair (the mocks model both, from the core's fixture). The core lexes the text up to the cursor,
finds the item the cursor is in and its position in that item, and answers from the vocabulary tables of §2.5; it never
runs the full parser on the incomplete item.

| Cursor position | Offered |
|---|---|
| start of an item (or after its label) | `lap`, `hr<`; ADR-0008: duration variables, number variables as `$n x(` |
| after a number at an end position (`10|`) | the units `s m h kj` and `x(` (`10m`, `10kj`, `10x(`); ADR-0010: `..` for a rep range (`3..`) |
| after a step's end, a target not written yet | `ramp`, `free`, after a number `%`, `w`, `bpm`; ADR-0008: power variables |
| after `ramp` | ADR-0008: `auto`, % FTP variables (numbers are typed, not offered) |
| after the target | cadence `rpm` after a number; `!lower`, `!cap`, `!coach` only on an `hr<` step |
| after a count at an item's start (`3..|`, `3..10|`) | ADR-0010: `x[hr>=` after the upper number (`10x[hr>=`); after a variable upper end, ` x[hr>=` with its space (`$hi x[hr>=`); number variables after `..` |
| inside `[ ]` of a repeat | `!last`; after a sign and a number `w`, `%`; ADR-0008: `+$p` with watts or % variables; ADR-0010: `hr>=` only in a **ranged** repeat's options and only if it has none yet (offered first, as canonical printing puts it first), and after `hr>=` bpm variables |
| after `$` | variables declared **above** the cursor whose type fits the position (ADR-0008 §2.6, §2.7); in a declaration, only those above it |
| inside a label, a comment, or a number's digits | nothing |

Variables not in scope (declared below, or of another type) are never offered; a block after the declarations sees
all of them. Options follow table order; a typed prefix filters them (CodeMirror's own filter on the core's options).

- **Rejected:** a vocabulary exported to TS and matched there: the position rules are grammar (which unit after which
  number, which flag on which step), and two copies would drift. **Cost, stated:** one IPC call per completion request;
  Tauri `invoke` answers locally, and CodeMirror's async source drops a stale answer when the text moves on.
- **Cost per keystroke, budgeted.** A keystroke costs one `parse_dsl_report` (linear in the text; plus at most 3
  fix-check parses, only while the text does not parse) and, while the completion list is open or a word is being
  typed, one `dsl_complete` (it lexes only up to the cursor). Budget, measured in release on the owner's machine: for a
  large realistic workout (500 items, nested repeats, ADR-0008 declarations; the built-ins are under 40) the parse with
  its fix checks within **4 ms** and `dsl_complete` within **1 ms**; for the parser's stress size (256 KiB of DSL text,
  as its property tests use) within **50 ms**. **Coalescing, not a timer:** the view keeps at most one parse in flight;
  keystrokes made meanwhile only move the draft, and when the answer returns the latest draft is sent if it differs
  (the stale answer is dropped by `parsed` as today). So a slow parse never queues, and a fast one adds no delay.
  **The budget includes serialising the answer and the IPC**, measured end to end from `invoke` to the answer in the
  page, with up to 3 fixes each carrying its `result` tree and text. To keep that bounded at the stress size, fixes
  are offered only for texts up to **64 KiB** (about 2 000 items, 4 × the large workout above): a longer text that does
  not parse gets the core's message and no fix, so the answer at 256 KiB stays one error and no trees. **Rejected:**
  sending a fix's result lazily on acceptance: a round-trip after the click brings back the fix-then-type race
  that §2.2's synchronous commit removes.
  CodeMirror's completion source is one-in-flight with abort by itself. The announcement waits for `GROUP_MS` (§2.7).
  If the benchmark misses the budget, the implementation PR says so with its numbers and is a design question, not a
  silent debounce.
- **Order with ADR-0008 and ADR-0010:** this ADR can land before or after their implementations; whichever lands
  second adds the rows marked ADR-0008 or ADR-0010 above to the tables and the tests. ADR-0010 is implemented after
  ADR-0008 (its §4.1), so its variable forms (`$lo..$hi x`) come with whichever of the two is later.

### 2.7 Keyboard and screen readers

- Completion opens while typing a word (CodeMirror's `activateOnTyping`) and on **Ctrl+Space**; the default extra Mac
  bindings Option-\` and Option-i are not bound: on the macOS US layout they are the grave and circumflex dead keys,
  and other layouts map them to characters too, which the rider may need in a label. If macOS takes Ctrl+Space for "select
  the previous input source", typing still opens the list.
- In the list: ↑ / ↓ / Page Up / Page Down move, **Enter accepts**, **Escape closes** (and only that: Escape outside
  the list does nothing in the text box). **Tab never accepts** and is not captured: Tab and Shift+Tab move focus as
  today (no keyboard trap, WCAG 2.1.2), so the text box stays one tab stop between the outline form and the Summary.
- Lint keys: `F8` goes to the next diagnostic, `Mod-Shift-m` opens CodeMirror's diagnostics panel (a listbox of
  diagnostics with their fixes). Neither collides with the editor's only shortcut, Cmd/Ctrl+Z (§5.9).
- **The message under the text box stays**, as the box's accessible description (`aria-describedby`): ⚠, the core's
  message, line and column, then "Did you mean ramp?" and a **Change to ramp** button per fix (32 px default, §18.4);
  the copied problem line can go, since the text itself is now marked. The underline is never the only cue (colour
  plus the marker and the message, §6.3).
- **Announcements:** when a burst of typing ends (a 500 ms pause, `GROUP_MS`) and the first error or its fixes differ
  from the last announced ones, the message is announced once through a polite live region ("Unknown token 'rmap'.
  Did you mean ramp?"). Not on every keystroke, and nothing is announced when the text becomes valid again besides
  the error leaving the description. The completion list is announced by CodeMirror's combobox semantics.

### 2.8 Lint beyond errors

- **Compile warnings** (`LongSprint`, `LowReserve`, `UnreachableRecovery`, …) are underlined on their block as
  warnings (§2.3) and stay in the Summary. **Compile errors** are drawn the same way as errors, ADR-0008's
  (`AutoRampNoNeighbour`, `AutoRampFacing`, …) and ADR-0010's: `PinAcrossRangedRep` on the pinned ramp (its node),
  `TriggerNotWatched` and `TriggerOutsideRange` on the ranged repeat's head (`3..10x[hr>=170, !last]`). They carry no
  fix: the message's example power ("e.g. 'ramp 120%-40%'") is an illustration, and which power the rider wants is
  theirs to write. **#627** (ERG blocks the slew cannot reach, pending the owner's answer on
  #585) needs nothing here: if it becomes a compile warning, it is drawn the same way.
- **Unused variable (ADR-0008 §2.6):** allowed and kept, so a **hint**, not a warning: faint, never in the Summary,
  never blocking; "$x is declared but not used", with the fix "Remove $x = …" (the declaration's line). A variable used
  only by another unused declaration is unused too, reported on each.
- **No style lint** (non-canonical spacing, `90s` vs `1m30s`): nearly every typed text would carry it, and a "format"
  fix through `print_dsl` would delete the rider's comments and blank lines (docs/DSL.md "Canonical printing": comments
  are not kept). An outline edit already prints the canonical text; that stays the only reformatting.

### 2.9 DESIGN §5.7 amendment (applied in the implementation PR)

The paragraph "**The editor in direction A**" changes where it says the problem's line is underlined "since a
textarea cannot mark a range of its own text": the text view is a CodeMirror editor; the problem's span, compile
warnings and errors (on their block's head) and hints are marked in the text; the message under it keeps the core's
message, line and column, and adds "Did you mean …?" with a button per checked fix; completion as §2.6 and §2.7. The
undo-group rules gain: a fix is a step of its own, committed at once from the core's checked parse; an accepted
completion is typing. **Parsing while typing changes:** today `edit` in `WorkoutEditor.tsx` sends one `parse_dsl` per
keystroke; it becomes one parse in flight at a time, the latest draft sent when the answer returns if it differs
(§2.6 "Cost per keystroke"), and §5.7 says so where it describes the text being read.

## 3. Alternatives considered

Listed in place: the textarea with an overlay, Monaco and Ace (§2.1); CodeMirror's own history (§2.2); a TS grammar or
vocabulary, a combined check command, spans by id (§2.3); error recovery (§2.4); Jaro-Winkler, Levenshtein, a
hand-written distance, unchecked fixes (§2.5); a style lint and a format fix (§2.8).

## 4. Consequences

- The text view marks problems in place and offers one-click, checked fixes; completion knows the grammar because it
  is the grammar's own tables.
- The contract changes: `DslParse.parsed` gains `spans` and `hints`, `DslProblem` gains `fixes` (each with its parsed
  `result`), `WorkoutDetail` gains `spans`, and a new command
  `dsl_complete`; `just bindings` regenerates `app/src/bindings.ts`, and the mocks' fixture
  (`crates/crank-core/tests/fixtures/editor.json`) gains typo drafts and completion answers generated by the core.
- The app gains CodeMirror (§2.1 size); editor tests move from `fireEvent.change` to the `setText` seam plus design
  harness typing.
- Implementation issues are filed on acceptance: (a) domain and core: vocabulary tables, spans, suggestions, checked
  fixes, hints (Rust, contract); (b) core: `dsl_complete`; (c) app: CodeMirror in place of the textarea with today's
  behaviour unchanged (controlled view, lock, undo); (d) app: diagnostics, fixes, completion, announcements, design
  scenes and e2e.

## 5. Test plan

**Rust (`crates/crank-domain/tests/`):**

- **Typo corpus** `tests/fixtures/dsl_typos.json`: text → expected fixes, at least: `rmap`, `rapm`, `fre`, `fee`,
  `lpa`, `!lst`, `!cpa`, `!lowr`, `90rmp`, `140bmp`, `4mm`, `10kjj`, `95%%`, `75-50%`, the ADR-0008 and ADR-0010 rows
  of §2.5 (`hr=>170`, `hr>170`, `5..5x[hr>=170]`, `$lo..$hix`), and the pinned `hr>` cases where both comparison
  keywords are one edit away: `3..10x[hr>170](…)` suggests only `hr>=170` (the options table has no `hr<`), and
  `hr>118 2m..4m 40%` at a step end only `hr<118` (the end table has no `hr>=`), so the position filter, not a tie
  rule, decides; and the
  **no-suggestion** cases that must stay silent: `5q`, `5m xyz`, `5m w`, `5m 50% hello`, `10 50%`, `5m banana`,
  upper-case keywords that are already valid (`RAMP`, no error), and ADR-0010's `3..10x(…)`, `10x[hr>=170](…)`,
  `[hr<120]` (message, no fix).
- **Every offered fix is checked** (property, 256 cases over generated valid documents with one generated typo: a
  swap, a dropped or doubled letter in a keyword or unit): applying each fix's edits (sorted, non-overlapping, original
  coordinates) gives text that parses; and no fix is offered for a word farther than the threshold from every candidate.
- **The parser and the tables agree:** every table entry, written in its position, parses; every keyword the parser
  accepts is in a table (a test walks the tables against the golden texts of #68).
- **Spans:** for generated valid documents, `spans.len()` equals the node count in pre-order, each span lies inside the
  text and starts at the node's end token or count; `parse_dsl_report`'s nodes equal `parse_dsl`'s.
- **Completion tables:** one test per row of §2.6 (cursor context → options), variables in scope only, `hr>=` never in
  a fixed repeat's options nor twice, and
  **completeness:** for every keyword or unit token in a generated valid text, the text cut inside that token and
  completed at the cut offers that token.
- **Hints:** unused variable reported, chained unused reported on each, the Remove fix parses.
- **Benchmark** (`crates/crank-domain/examples/dsl_editor_bench.rs`, `just bench-editor`, release): the budget of
  §2.6 on the 500-item workout, a 64 KiB text and the 256 KiB text, for a valid text, an invalid one with 3 fix
  candidates, and completion at the end of the text, timing the core's work plus `serde_json` serialising of the DTO
  answer; the IPC leg is timed once in the real app (sim, `invoke` to answer in the page) on the same texts; numbers
  reported in the implementation PR. It is not a CI gate (shared runners
  make timing gates flaky); the CI-checked bounds are deterministic: at most 3 check parses per answer, and
  `dsl_complete` reads no token past the cursor (a test with a lexer that counts tokens).
- **The 64 KiB cut-off:** a text of 64 KiB that does not parse gets its fixes; one byte more gets the message and no
  fix (so no `result` tree is serialised past it).

**App (Vitest):** model: a fix is one undo step committed by `fixed` alone (no parse); the fix button and the
diagnostic action call `fixed` and never `edit`, and the view follows with an external transaction that starts no
typing burst (an update listener spy sees no `edit` call); the previous answer's fix buttons are gone as soon as the
draft moves on; keys typed right after it form
a new step (fix, type, Cmd+Z leaves the fix; Cmd+Z again removes it); a fix whose `base` is no longer the draft is
refused; Save is enabled right after a fix with no "Reading the text…"; an accepted completion joins the burst; `load` and
`applied` keep spans, and after an outline move a compile warning is underlined on the moved block; the view sends no
change when draft equals document and none during a composition; `historyUndo` / `historyRedo` reach the model; the controlled view keeps
the cursor outside an external change (prefix/suffix property); the lock makes the view read-only and disables fixes;
spans survive undo; the mocks model `dsl_complete`'s `InvalidInput` refusals as the core does.

**Design harness (`just design`, `just design-webkit`):** scenes for an error underline with its fix, the completion list
open, a warning on a block, the locked text; layout invariants at every viewport; axe; focus audit (Tab never lands in
the popup).

**e2e (real app, sim):** type `rmap` in a step, accept "Change to ramp", save, ride the saved workout.

**Owner check (batched with the app PR):** in the built macOS app: typing, Cmd+Z by key and by Edit ▸ Undo, a dead
key, dictation, VoiceOver reading the diagnostic.

## 6. Open questions

None blocking; the owner list above holds the decisions.

| Question | Issue | Milestone |
|---|---|---|

## 7. References

Checked 2026-11-27.

- CodeMirror 6 reference manual (lint, autocomplete, view): https://codemirror.net/docs/ref/ ; package versions and
  licences from the npm registry (`npm view`), bundle sizes measured with esbuild 0.25.0.
- strsim-rs 0.11.1: https://github.com/rapidfuzz/strsim-rs (`osa_distance`: "Like Levenshtein but allows for
  adjacent transpositions. Each substring can only be edited once.")
- rustc `compiler/rustc_span/src/edit_distance.rs`: restricted Damerau-Levenshtein; `find_best_match_for_name` limit
  `max(lookup_len, 3) / 3`, an exact case-insensitive match first.
- WAI-ARIA APG, Combobox pattern: https://www.w3.org/WAI/ARIA/apg/patterns/combobox/ ; WCAG 2.1 SC 2.1.2 No Keyboard Trap.
- In repo: `crates/crank-domain/src/workout/{dsl,model,compiled,compile}.rs`; `crates/crank-core/src/{core,dto}.rs`;
  `app/src-tauri/src/commands.rs`; `app/src/editor/{WorkoutEditor.tsx,model.ts}`; `app/src/views/{editorFixture,workoutsBackend}.ts`;
  `app/e2e/specs/mvp-selectors.mjs`; DESIGN §5.7, §5.9; ADR-0008; ADR-0010 (PR #737).
