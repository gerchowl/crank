# Cross-cutting issues (seen in ≥ 2 spikes)

- [ ] **X1 AI rewrite accept loses or misplaces user edits** (React, Svelte, Solid). The proposal is built from a snapshot or path
  taken at request time; edits remain possible while the rewrite is pending or the diff is open; accept applies the stale result.
  → Lock editing during pending/diff, and on accept verify that the target node ids and content hash are unchanged, otherwise re-request.
- [ ] **X2 Physiology computed in the UI** (all three): Δ% to target, lap compliance (different formula than the backend),
  per-lap averages over *planned* rather than actual block times, τ "slower" threshold (1.1× / 1.15×), W′bal floor, ±5 % target band,
  recovery curve evaluation. → Contract additions (see `backend.md` B1–B3); the UI only displays.
- [ ] **X3 Per-ride state not reset on new workout** (React record + nudges; Solid marker drift). → One `rideSession` scope
  recreated on `ride.load`.
- [ ] **X4 Unvalidated localStorage crashes the app** (React snippets, Svelte snippets). → Schema-validate all persisted data;
  top-level error boundary.
- [ ] **X5 structuredClone on reactive proxies** (Svelte, Solid). → A single `snapshot()` helper; lint/grep rule against direct clone.
- [ ] **X6 Finished ride doesn't reach Review** automatically (React, Svelte). → Auto-navigate on `state: finished`.
- [ ] **X7 1024×768 not usable** (React outline clipped, Svelte top bar off-screen, Solid fonts below 64 px). Device layout
  doesn't follow resize (all). → Responsive top bar and layout switching on resize; screenshot tests at 1024.
- [ ] **X8 Layout-editor state bugs** (React pageIdx after reset, Svelte Reset ignores Cancel, Solid kind change keeps metrics,
  heatmap "—" metric). → Layout editing as a transactional draft with validation per widget kind.
- [ ] **X9 No unit tests for editor tree ops, undo/redo, DSL↔tree sync** (all). Every editor bug would have been caught.
- [ ] **X10 Ride-screen layout shift** (Solid header jumps on pause; reported by user). → Fixed-width controls, tabular numerals.
- [ ] **X11 Widget settings seeded from props once** (React `useState(widget.spanS)`, Svelte prop seeding) → changes only
  apply after leaving edit mode.
