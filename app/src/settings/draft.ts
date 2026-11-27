// The settings-draft primitive, as far as Settings → Athlete model needs it (#90 builds the rest: persisted shapes,
// their parsers and notices). A form edits a draft, never the saved value: its controls read `read()` on every render,
// `set()` changes the draft only, `save()` validates and writes once, `cancel()` drops the draft. A refused validation
// or write leaves the saved value as it was and the draft open, with the reason.
import { useState, useSyncExternalStore } from "react";

/** Checks a draft before it is written: `null` when it may be, else why not (shown under the form). */
export type Validate<T> = (draft: T) => string | null;
/** Writes a valid draft: `null` once written, else the refusal's text (the draft stays open). */
export type Write<T> = (draft: T) => Promise<string | null>;

export class Draft<T> {
  private draft: T;
  private problem: string | null = null;
  private busy = false;
  private readonly listeners = new Set<() => void>();
  private view: { draft: T; error: string | null; saving: boolean };

  constructor(
    private saved: T,
    private readonly validate: Validate<T>,
  ) {
    this.draft = saved;
    this.view = { draft: saved, error: null, saving: false };
  }

  /** The draft, the last refusal, and whether a save is under way; one object per change. */
  read(): { draft: T; error: string | null; saving: boolean } {
    return this.view;
  }
  /** The value the form was opened on, or last saved. */
  savedValue(): T {
    return this.saved;
  }
  /** Changes the draft only. */
  set(next: T): void {
    this.draft = next;
    this.problem = null;
    this.emit();
  }
  /** Drops the draft: it is the saved value again. */
  cancel(): void {
    this.draft = this.saved;
    this.problem = null;
    this.emit();
  }
  /** Validates, then writes once; `true` when written. A second save while one is under way does nothing. */
  async save(write: Write<T>): Promise<boolean> {
    if (this.busy) return false;
    const draft = this.draft;
    this.problem = this.validate(draft);
    if (this.problem !== null) {
      this.emit();
      return false;
    }
    this.busy = true;
    this.emit();
    const refused = await write(draft);
    this.busy = false;
    this.problem = refused;
    if (refused === null) this.saved = draft;
    this.emit();
    return refused === null;
  }
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
  private emit() {
    this.view = { draft: this.draft, error: this.problem, saving: this.busy };
    for (const l of this.listeners) l();
  }
}

/** A draft for the life of the calling form, opened on `saved` once; the form re-renders on each change. */
export function useDraft<T>(
  saved: () => T,
  validate: Validate<T>,
): [Draft<T>, ReturnType<Draft<T>["read"]>] {
  const [draft] = useState(() => new Draft(saved(), validate));
  const view = useSyncExternalStore(
    (l) => draft.subscribe(l),
    () => draft.read(),
  );
  return [draft, view];
}
