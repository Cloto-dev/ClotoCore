/**
 * Turning the stored severity into something a person reads, and deciding what
 * is allowed to interrupt them.
 *
 * The kernel stores the RFC 5424 identifier — the same eight values MCP logging
 * carries — and never a display scale. Three levels are what a reader actually
 * chooses between, so the reduction happens here, at the edge, once. Storing the
 * three would have meant keeping a table against the eight by hand, which is the
 * arrangement that goes quietly out of step.
 *
 * **Nothing in this file may be used to decide whether an item is shown.** The
 * threshold answers one question — does this interrupt — and the badge and the
 * list are deliberately not among its callers. A reader who narrows the
 * threshold is asking for fewer interruptions, not for a quieter account of what
 * their agents are stuck on; the two get confused easily, and the failure is
 * silent in the worst direction (an agent waits, the badge reads zero).
 */

import type { NotificationSeverity } from '../services/api';

/**
 * RFC 5424 order: 0 is the most severe. Written as a `Record` over the union so
 * a severity added to the type fails to compile here rather than sorting as
 * `undefined` — which, being neither greater nor less than anything, would make
 * an item quietly stop qualifying rather than loudly break.
 */
const SEVERITY_RANK: Record<NotificationSeverity, number> = {
  emergency: 0,
  alert: 1,
  critical: 2,
  error: 3,
  warning: 4,
  notice: 5,
  info: 6,
  debug: 7,
};

/** What a reader picks between. */
export type DisplayLevel = 'high' | 'medium' | 'low';

const DISPLAY_ORDER: Record<DisplayLevel, number> = { high: 0, medium: 1, low: 2 };

/**
 * The band an identifier falls in.
 *
 * `error` and louder is high; `warning` is medium; everything quieter is low.
 * The cut is at `error` because that is where the kernel's own derivation puts
 * a destructive or unclassified tool call — the two cases a reader most wants
 * in front of them.
 */
export function displayLevel(severity: NotificationSeverity): DisplayLevel {
  const rank = SEVERITY_RANK[severity];
  if (rank <= SEVERITY_RANK.error) return 'high';
  if (rank <= SEVERITY_RANK.warning) return 'medium';
  return 'low';
}

/**
 * Whether an item at this severity is allowed to interrupt at this threshold.
 *
 * Used for one thing: marking, and later raising, the card that appears without
 * being asked for. It is never consulted about whether an item exists.
 */
export function interrupts(severity: NotificationSeverity, threshold: DisplayLevel): boolean {
  return DISPLAY_ORDER[displayLevel(severity)] <= DISPLAY_ORDER[threshold];
}

/** Where the reader's choice is kept. Per-browser; nothing else reads it. */
const THRESHOLD_KEY = 'cloto-notification-threshold';

const DISPLAY_LEVELS: readonly DisplayLevel[] = ['high', 'medium', 'low'];

function isDisplayLevel(value: unknown): value is DisplayLevel {
  return typeof value === 'string' && (DISPLAY_LEVELS as readonly string[]).includes(value);
}

/**
 * The reader's threshold, defaulting to `medium`.
 *
 * A stored value that is not one of the three is treated as absent rather than
 * trusted: this is browser storage, so it can hold anything a previous version
 * or another tab left behind, and an unrecognised threshold compared with
 * `undefined` silently stops everything from interrupting.
 */
export function loadThreshold(): DisplayLevel {
  try {
    const stored = localStorage.getItem(THRESHOLD_KEY);
    return isDisplayLevel(stored) ? stored : 'medium';
  } catch {
    // Private windows, cleared site data, storage disabled: none of those are
    // reasons to fail to render a bell.
    return 'medium';
  }
}

export function saveThreshold(threshold: DisplayLevel): void {
  try {
    localStorage.setItem(THRESHOLD_KEY, threshold);
  } catch {
    // Same as above: the preference is a convenience, not state anything depends on.
  }
}
