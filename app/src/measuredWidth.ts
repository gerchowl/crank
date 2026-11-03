// The measured display width as a small external store (#313), kept apart from the Tauri calls so it is tested
// with plain fakes. It owns the two things a bare `useEffect` got wrong or could not show:
// - Reads carry a sequence number and only the newest one counts: a slow earlier read that resolves after a
//   newer one (the window moved while the mount read was still out) must not overwrite it.
// - When the last subscriber leaves, the watch is stopped and every read still in flight is made stale, so a late
//   answer changes nothing. (A React state update after unmount is a silent no-op, so this guard is only
//   observable here, on the store.)

export interface MeasuredWidth {
  /** The latest width in mm; null while unknown or before the first answer. */
  get(): number | null;
  /** Start reading and watching on the first subscriber; stop on the last one leaving. */
  subscribe(listener: () => void): () => void;
}

/**
 * `read` answers the width (null when unknown), `watch(refresh)` calls `refresh` whenever the window may be on
 * another display and returns its unsubscribe.
 */
export function createMeasuredWidth(
  read: () => Promise<number | null>,
  watch: (refresh: () => void) => () => void,
): MeasuredWidth {
  let value: number | null = null;
  let seq = 0;
  let stop: (() => void) | undefined;
  const listeners = new Set<() => void>();

  const refresh = () => {
    const mine = ++seq;
    void read().then((v) => {
      if (mine !== seq) return; // a newer read was started, or the last subscriber left
      if (v === value) return;
      value = v;
      for (const l of [...listeners]) l();
    });
  };

  return {
    get: () => value,
    subscribe(listener) {
      listeners.add(listener);
      if (listeners.size === 1) {
        refresh();
        stop = watch(refresh);
      }
      return () => {
        listeners.delete(listener);
        if (listeners.size === 0) {
          stop?.();
          stop = undefined;
          seq++; // reads still out are stale now
        }
      };
    },
  };
}
