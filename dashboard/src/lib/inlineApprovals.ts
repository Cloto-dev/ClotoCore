/**
 * Which approvals are being asked inside a conversation right now.
 *
 * A question an agent asks is drawn in two places: inside their conversation,
 * as the mock draws it, and — for an agent you are not looking at — over the
 * window by the deck. The deck reads this set so the same question is never
 * asked twice on one screen: the conversation that draws it says so here, and
 * the deck leaves it alone while it does.
 */

const inline = new Set<string>();
const listeners = new Set<() => void>();
let version = 0;
let snapshot: ReadonlySet<string> = new Set();

function changed() {
  version += 1;
  snapshot = new Set(inline);
  for (const l of listeners) l();
}

export function markInline(approvalId: string): void {
  if (inline.has(approvalId)) return;
  inline.add(approvalId);
  changed();
}

export function unmarkInline(approvalId: string): void {
  if (!inline.delete(approvalId)) return;
  changed();
}

/** For `useSyncExternalStore`: the current set, replaced on every change. */
export function inlineSnapshot(): ReadonlySet<string> {
  return snapshot;
}

export function subscribeInline(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Test seam. */
export function resetInline(): void {
  inline.clear();
  changed();
}

export function inlineVersion(): number {
  return version;
}
