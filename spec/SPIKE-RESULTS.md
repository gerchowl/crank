# UI framework spike results (2026-10-13)

Three Opus agents built the same scope (spec/SPIKE-BRIEF.md) in React 19, Svelte 5 and SolidJS against
`@neo/shared`. Each was then reviewed by an independent adversarial agent (Playwright probes) and
measured identically (production build, headless Chromium, 60× ride, forced GC).

| | React 19 + Compiler | Svelte 5 (runes) | SolidJS |
|---|---|---|---|
| Spec / correctness / robustness / clarity | 8 / 5 / 5 / 8 | 8 / 5 / 5 / 8 | 7 / 4 / 5 / 7 |
| Critical bugs | 0 | 0 | 2 |
| Framework-specific silent runtime bugs | 0 | 2 | 2 |
| LOC (src) | 5.7k | 6.9k | 6.8k |
| JS gzip | 154 kB | 102 kB | 93 kB |
| Heap after GC | 6.0 MB | 5.6 MB | 5.2 MB |
| Ride: leaks / rates / fps | none / exact / 60 | none / exact / 60 | none / exact / 60 |
| Lint/type catches during build | 8 | 6 | 7 |
| Tokens to build | 28M | 34M | 31M |

**Recommendation: React** (Svelte a close second). Quality ties with Svelte. React's remaining bugs are
ordinary app logic, while Svelte and Solid both produced silent framework-specific runtime bugs (proxy
cloning and mutation, untracked reads) that clean checks didn't catch. Bundle size is irrelevant under Tauri.

## Notable bugs per spike
- **React:** the review showed the previous ride (module-level record never reset); AI accept resurrected
  undone edits; the DSL was editable during a diff; moving a repeat together with its own child duplicated
  nodes; finishing a ride didn't navigate to Review; a stale coach nudge; the layout pageIdx broke after reset;
  no error boundary.
- **Svelte:** AI accept replaced the wrong nodes after intervening edits; a stuck `#restoring` flag corrupted
  undo/redo; an invalid DSL draft masked outline edits; the top bar overflowed at 1024 px; Reset ignored
  Cancel; the first `??=` filter push bypassed the proxy; empty repeats printed as invalid DSL.
- **Solid:** the profile and summary never updated after edits (untracked read); "Ride this" crashed
  (structuredClone on store proxy); AI accept dropped edits; Auto theme mixed light surfaces with dark series;
  shortcuts died after focusing a checkbox; the "you are here" marker drifted after skips.

## Rules for the production UI (derived from all three)
1. Lock the editor while an AI rewrite is pending or a diff is open; on accept, verify the target nodes are unchanged.
2. Unit-test tree ops, undo/redo and DSL ↔ tree sync (including invalid drafts). Every editor bug above would have been caught.
3. Reset all per-ride state on workout load (record, nudges, selection, page index).
4. No physiology in the UI. Add per-lap stats and explicit thresholds (τ "slower" factor, W′bal floor, target band)
   to the contract.
5. Validate all persisted data (layouts, snippets); top-level error boundary.
6. Ride screen: zero layout shift, tabular numerals, top bar usable at 1024 px, auto-navigate to Review on finish,
   device layout follows window size.
7. Never `structuredClone` reactive state directly; snapshot through a single helper.
