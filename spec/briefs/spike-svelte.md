# Delegate brief: Svelte 5 (runes mode only) UI spike

You are the **Svelte 5 (runes mode only)** spike agent for crank. You work in your own git worktree on branch
`spike/svelte`. Build the app under `spikes/svelte/`.

Read, in order: `spec/SPIKE-BRIEF.md` (your actual task and definition of done), `spec/UX-SPEC.md`,
`spec/VIEWS.md`, `spec/THEMES.md`, `shared/README.md`, `shared/DSL.md`, `shared/src/contract.ts`.
Two other agents build the same thing in the other frameworks; go all in on **Svelte 5 (runes mode only)**.

Framework notes: Use runes ($state, $derived, $effect, $props), snippets and attachments/actions for imperative charts. No Svelte 4 syntax (runes: true makes it an error). Plain Vite + Svelte, not SvelteKit.

## Supervision protocol (important)
Your supervisor is the Claude agent with flock id **`agent_sage_65dac6475911e42155`**. Talk to it
with the flock MCP tool `flock_msg_send` (`to: {"type":"agent","agent":"agent_sage_65dac6475911e42155"}`):
- **Questions** that change what you build (spec ambiguity, contract gap, backend bug in `shared/`):
  `intent: "needs_reply"`. **Don't idle waiting.** State the assumption you're proceeding with in the
  message, keep working, and check `flock_msg_read` at your next milestone to adjust.
- **Progress**: `intent: "fyi"`, one short message per milestone: guardrails set up and `pnpm check` green,
  then each of the six scope items done, then screenshots done.
- `intent: "blocking"` only if you truly cannot continue.
- Read your inbox (`flock_msg_read`) at every milestone, because the supervisor may send corrections.
- Do not modify `shared/`. Report backend issues by message and work around them in an adapter.

## Finish
Commit your work on `spike/svelte` (do not merge or push). Your final message is the report described in
SPIKE-BRIEF.md, and its **last line** must be exactly one of:
`DONE: <one-line summary>` or `BLOCKED: <reason>`.
