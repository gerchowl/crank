# ADR-0010: Workout text editor: diagnostics with fixes, "did you mean", and completion from the core

- **Status:** Proposed (for owner review); not implemented
- **Date:** 2026-11-27
- **Issue:** #732
- **Related:** DESIGN §5.7 (editor, undo groups, outline edits #653), §5.9 (desk keys), `docs/DSL.md` (grammar, errors),
  ADR-0008 (`auto`, `$variables`, its errors §2.3 / §2.14), ADR-0001 §3–4 (domain contract, bindings), #627 (slew
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
   grammar, no keyword list and no matching (§2.3, §2.6).
4. **The first parse error only**, as today: the parser stays fail-fast, with no error recovery (§2.4).
5. **"Did you mean" uses `strsim` 0.11.1 `osa_distance`** (adjacent swaps count as one edit), compared case-insensitively
   against only the words valid in that position, with rustc's threshold (at most ⌊max(len, 3) / 3⌋ edits, so one edit
   for words of 3 to 5 letters and two for 6 to 8), at most 3 suggestions, all at the smallest distance (§2.5).
6. **A fix is offered only when the core has checked it**: applying it makes the text parse, or moves the first error
   past the fix. Ambiguous errors (`10 50%`: seconds, minutes, kJ or a repeat?) get no fix, only the message (§2.5).
7. **Completion comes from the core by cursor context** (§2.6 table). It opens while typing a word and on Ctrl+Space;
   **Enter accepts, Escape closes, Tab never accepts** (Tab keeps moving focus out of the text box) (§2.7).
8. **Lint beyond errors:** compile warnings are underlined on their block (and #627's slew warning, once decided, comes the
   same way); ADR-0008's unused variable is a **hint** that never blocks saving, with a "Remove" fix; **no style lint** and
   no "format" fix, because canonical printing drops comments (§2.8).
9. **A fix is one undo step of its own**; accepting a completion is typing and joins the burst (§2.2).
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
  outline edit one step). The macOS Edit ▸ Undo path is checked as in §2.1; if it reaches CodeMirror as a
  `historyUndo` input, the text component routes it to the model, never to the DOM.
- **The model's `draft` is the truth; the view follows it.** A change made in CodeMirror (a keystroke, a completion)
  calls `edit(text, at)` as today. A change that comes from the model (undo, redo, an outline edit, Accept of an AI
  rewrite) is dispatched into the view as one replacement of the changed region: the common prefix and suffix of the
  old and new text are kept, so the cursor stays put when it lies outside the change. That trim is a few lines with a
  property test (old text with the region replaced equals the new text); a diff library would only improve where the
  cursor lands after an edit made elsewhere, so it is not worth a dependency (`@codemirror/merge`'s `diff` was
  considered).
- **Undo steps:** accepting a completion is typing (it joins the burst like the keys it replaces); **a fix is a step of
  its own** (model: `replaced(s, text)`, a commit that ends the burst, like an outline edit), so Cmd+Z after a fix
  undoes exactly the fix.
- **The AI rewrite lock** (#112, `model.ts` `lock`): the view becomes read-only (`EditorState.readOnly` and
  `EditorView.editable` in a compartment), fix buttons and completion are disabled, selection and copy still work.
  The model already refuses every edit while locked; this keeps the view from showing keys the model drops.

### 2.3 The diagnostics contract

Diagnostics stay the core's: spans, severity and message come from Rust; TypeScript only places them.

**Domain** (`crank-domain`, new module `workout/lint.rs`, no new public behaviour of `parse_dsl`):

```rust
/// A replacement of a UTF-16 span of the text (length 0 = insertion).
pub struct DslEdit { pub offset: usize, pub length: usize, pub insert: String }
/// A fix the core has checked (§2.5): what the UI shows after "Did you mean", and its edits.
pub struct DslFix { pub label: String, pub edits: Vec<DslEdit> }
/// The UTF-16 span of each node's head, in pre-order of the parsed nodes: a step from its end token to its last
/// attribute (its label excluded); a repeat its count and options (`3x[+5w]`, `$n x`), not its body.
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
```

**Compile issues get their place by node:** the editor already compiles the committed tree; an issue's `node_id` is
found in that tree in pre-order, and the span at that index is the one underlined. The spans are kept beside the tree
in the model's snapshot (`saved: { nodes, text, spans }`, and so in `past` and `future`), so undo restores them with
the text and no re-parse is needed. Issues without a node (`EmptyWorkout`, `MissingCooldown`, `InvalidFtp`) stay in
the Summary only. While the draft does not parse, only the parse error is drawn (the compile issues describe the saved
tree, whose spans no longer match the text); the Summary keeps them as today.

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
`$names` declared above the position with a type that fits it). The parser matches against these tables, and the
suggester and completion (§2.6) read the same ones, so a keyword cannot exist in one and not the others.

**Matching:** `strsim::osa_distance` (optimal string alignment: insert, delete, substitute, swap two neighbours; the
algorithm rustc's `edit_distance` uses), on the lower-cased word against each candidate valid where the error is.
A word that starts with a digit or a sign is a number with a unit (below); any other word is matched against the
keywords. A candidate is suggested when its distance is at most ⌊max(n, 3) / 3⌋, n the typed word's length in characters
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
| ADR-0008: the spelling refusals whose message shows the fix | that spelling | `$thr=95%` → `$thr = 95%`; `ramp 45-auto` → `ramp 45%-auto`; `ramp 45%-75%` → `ramp 45-75%`; `$a = $b-30s` → `$b - 30s` |

The core's `message` stays as it is (the TS-parity text, docs/DSL.md "Errors"); suggestions are not appended to it. The
UI writes "Did you mean ramp?" from the fix's `label`, and the fix's button reads "Change to ramp".

**A fix is checked before it is offered:** the core applies each candidate fix to the text and parses the result with
a throw-away id source; the fix is offered only if that text parses, or its first error starts after the fix's last
edit (the fix removed this error and the rider meets the next one). A candidate that fails the check is dropped,
not shown with a warning. That costs at most 3 extra parses, only when the text does not parse.

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
| after a number at an end position (`10|`) | the units `s m h kj` and `x(` (`10m`, `10kj`, `10x(`) |
| after a step's end, a target not written yet | `ramp`, `free`, after a number `%`, `w`, `bpm`; ADR-0008: power variables |
| after `ramp` | ADR-0008: `auto`, % FTP variables (numbers are typed, not offered) |
| after the target | cadence `rpm` after a number; `!lower`, `!cap`, `!coach` only on an `hr<` step |
| inside `[ ]` of a repeat | `!last`; after a sign and a number `w`, `%`; ADR-0008: `+$p` with watts or % variables |
| after `$` | variables declared **above** the cursor whose type fits the position (ADR-0008 §2.6, §2.7); in a declaration, only those above it |
| inside a label, a comment, or a number's digits | nothing |

Variables not in scope (declared below, or of another type) are never offered; a block after the declarations sees
all of them. Options follow table order; a typed prefix filters them (CodeMirror's own filter on the core's options).

- **Rejected:** a vocabulary exported to TS and matched there: the position rules are grammar (which unit after which
  number, which flag on which step), and two copies would drift. **Cost, stated:** one IPC call per completion request;
  Tauri `invoke` answers locally, and CodeMirror's async source drops a stale answer when the text moves on.
- **Order with ADR-0008:** this ADR can land before or after ADR-0008's implementation; whichever lands second adds the
  `auto` and variable rows to the tables and the tests, as the rows marked ADR-0008 above say.

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
  warnings (§2.3) and stay in the Summary. **#627** (ERG blocks the slew cannot reach, pending the owner's answer on
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
undo-group rules gain: a fix is a step of its own; an accepted completion is typing.

## 3. Alternatives considered

Listed in place: the textarea with an overlay, Monaco and Ace (§2.1); CodeMirror's own history (§2.2); a TS grammar or
vocabulary, a combined check command, spans by id (§2.3); error recovery (§2.4); Jaro-Winkler, Levenshtein, a
hand-written distance, unchecked fixes (§2.5); a style lint and a format fix (§2.8).

## 4. Consequences

- The text view marks problems in place and offers one-click, checked fixes; completion knows the grammar because it
  is the grammar's own tables.
- The contract changes: `DslParse.parsed` gains `spans` and `hints`, `DslProblem` gains `fixes`, and a new command
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
  `lpa`, `!lst`, `!cpa`, `!lowr`, `90rmp`, `140bmp`, `4mm`, `10kjj`, `95%%`, `75-50%`, the ADR-0008 rows of §2.5, and the
  **no-suggestion** cases that must stay silent: `5q`, `5m xyz`, `5m w`, `5m 50% hello`, `10 50%`, `5m banana`,
  upper-case keywords that are already valid (`RAMP`, no error).
- **Every offered fix is checked** (property, 256 cases over generated valid documents with one generated typo: a
  swap, a dropped or doubled letter in a keyword or unit): applying each fix gives text that parses, or whose first
  error starts after the fix; and no fix is offered for a word farther than the threshold from every candidate.
- **The parser and the tables agree:** every table entry, written in its position, parses; every keyword the parser
  accepts is in a table (a test walks the tables against the golden texts of #68).
- **Spans:** for generated valid documents, `spans.len()` equals the node count in pre-order, each span lies inside the
  text and starts at the node's end token or count; `parse_dsl_report`'s nodes equal `parse_dsl`'s.
- **Completion tables:** one test per row of §2.6 (cursor context → options), variables in scope only, and
  **completeness:** for every keyword or unit token in a generated valid text, the text cut inside that token and
  completed at the cut offers that token.
- **Hints:** unused variable reported, chained unused reported on each, the Remove fix parses.

**App (Vitest):** model: a fix is one undo step and an accepted completion joins the burst; the controlled view keeps
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
  `app/e2e/specs/mvp-selectors.mjs`; DESIGN §5.7, §5.9; ADR-0008.
