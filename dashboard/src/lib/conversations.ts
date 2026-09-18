// Conversations on the client (docs/CONVERSATIONS_DESIGN.md): which one is
// open for each agent, and how the list is grouped. The kernel owns the
// conversations themselves; this file owns nothing the kernel could.

import type { Conversation } from '../types';

const OPEN_KEY_PREFIX = 'cloto-open-conversation:';

/** The conversation the person had open for this agent, if the browser
 * remembers one. Nothing is verified here — the caller checks it still
 * exists and falls back to the list when it does not. */
export function rememberedConversation(agentId: string): string | null {
  try {
    return localStorage.getItem(OPEN_KEY_PREFIX + agentId);
  } catch {
    return null;
  }
}

export function rememberConversation(agentId: string, conversationId: string | null): void {
  try {
    if (conversationId) localStorage.setItem(OPEN_KEY_PREFIX + agentId, conversationId);
    else localStorage.removeItem(OPEN_KEY_PREFIX + agentId);
  } catch {
    // Storage is a convenience; the list is the fallback.
  }
}

export type ConversationGroup = 'today' | 'yesterday' | 'previous_7_days' | 'older';

const DAY_MS = 24 * 60 * 60 * 1000;

/** Midnight (local time) of the day containing `ms`. */
function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** Which sidebar group a conversation belongs to, by its newest activity,
 * measured in the viewer's local days: "yesterday" is the calendar day
 * before today, not the last 24 hours. */
export function groupOf(updatedAt: number, now: number): ConversationGroup {
  const today = startOfDay(now);
  if (updatedAt >= today) return 'today';
  if (updatedAt >= today - DAY_MS) return 'yesterday';
  if (updatedAt >= today - 7 * DAY_MS) return 'previous_7_days';
  return 'older';
}

export const GROUP_ORDER: readonly ConversationGroup[] = ['today', 'yesterday', 'previous_7_days', 'older'];

/** The list in display order: newest activity first, split into the four
 * groups, empty groups omitted. Archived conversations are never in the
 * sidebar, whatever the caller passed. */
export function groupConversations(
  conversations: readonly Conversation[],
  now: number,
): { group: ConversationGroup; items: Conversation[] }[] {
  const live = conversations.filter((c) => c.archived_at === null || c.archived_at === undefined);
  const sorted = [...live].sort((a, b) => b.updated_at - a.updated_at);
  const buckets = new Map<ConversationGroup, Conversation[]>();
  for (const c of sorted) {
    const g = groupOf(c.updated_at, now);
    const bucket = buckets.get(g) ?? [];
    bucket.push(c);
    buckets.set(g, bucket);
  }
  return GROUP_ORDER.filter((g) => buckets.has(g)).map((g) => ({ group: g, items: buckets.get(g) ?? [] }));
}

/** What a row shows when the kernel has not titled the conversation yet. */
export function displayTitle(c: Pick<Conversation, 'title'>, untitled: string): string {
  return c.title.trim() === '' ? untitled : c.title;
}
