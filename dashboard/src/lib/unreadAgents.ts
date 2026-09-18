/**
 * Which agents have something waiting for the person on the roster.
 *
 * The mark is a claim about *the person owing an agent an answer*, so only the
 * kinds that carry a question count. `notice` is deliberately not among them:
 * a rejected tool call raises one every time an agent runs, so counting notices
 * would put a mark beside every busy agent and the mark would stop meaning
 * anything. An item also stops counting once it is read or resolved, and one
 * with no `agent_id` belongs to no row.
 */

import type { NotificationItem, NotificationKind } from '../services/api';

/** The kinds a roster mark counts. Exported so a test can state the list. */
export const UNREAD_KINDS: readonly NotificationKind[] = ['approval', 'proposal'];

/** True when this one item is what puts a mark on its agent's row. */
export function isUnreadForAgent(item: NotificationItem): boolean {
  return (
    item.agent_id !== null && item.read_at === null && item.resolved_at === null && UNREAD_KINDS.includes(item.kind)
  );
}

/** The agents with at least one unanswered, unread question. */
export function unreadAgentIds(items: NotificationItem[]): Set<string> {
  const ids = new Set<string>();
  for (const item of items) {
    if (isUnreadForAgent(item) && item.agent_id !== null) ids.add(item.agent_id);
  }
  return ids;
}

/** The items whose mark this agent's row carries — what opening the chat reads. */
export function unreadItemsFor(items: NotificationItem[], agentId: string): NotificationItem[] {
  return items.filter((item) => item.agent_id === agentId && isUnreadForAgent(item));
}
