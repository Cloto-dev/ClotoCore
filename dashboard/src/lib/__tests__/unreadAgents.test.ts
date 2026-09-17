import { describe, expect, it } from 'vitest';
import type { NotificationItem } from '../../services/api';
import { UNREAD_KINDS, unreadAgentIds, unreadItemsFor } from '../unreadAgents';

function item(over: Partial<NotificationItem> = {}): NotificationItem {
  return {
    item_id: 'i1',
    kind: 'approval',
    severity: 'warning',
    agent_id: 'agent.a',
    title: 'May I run this?',
    body: null,
    created_at: '2026-09-17T00:00:00Z',
    read_at: null,
    resolved_at: null,
    decision: null,
    blocking: true,
    metadata: null,
    ...over,
  };
}

describe('the roster mark', () => {
  it('counts an unanswered, unread question, once per agent', () => {
    const ids = unreadAgentIds([
      item({ item_id: '1', agent_id: 'agent.a' }),
      item({ item_id: '2', agent_id: 'agent.a', kind: 'proposal' }),
      item({ item_id: '3', agent_id: 'agent.b', kind: 'proposal' }),
    ]);
    expect([...ids].sort()).toEqual(['agent.a', 'agent.b']);
  });

  it('does not count a notice — it is what an agent raises while it works, not a question', () => {
    // This is the test that fails if 'notice' is added to UNREAD_KINDS.
    const ids = unreadAgentIds([item({ item_id: '9', agent_id: 'agent.noisy', kind: 'notice' })]);
    expect(ids.has('agent.noisy')).toBe(false);
    expect(ids.size).toBe(0);
    expect([...UNREAD_KINDS]).toEqual(['approval', 'proposal']);
  });

  it('stops counting once the item is read, answered, or belongs to nobody', () => {
    const read = unreadAgentIds([item({ agent_id: 'agent.r', read_at: '2026-09-17T01:00:00Z' })]);
    expect(read.size).toBe(0);

    const resolved = unreadAgentIds([item({ agent_id: 'agent.s', resolved_at: '2026-09-17T01:00:00Z' })]);
    expect(resolved.size).toBe(0);

    const orphan = unreadAgentIds([item({ agent_id: null })]);
    expect(orphan.size).toBe(0);
  });

  it('hands back exactly the items whose mark an agent carries', () => {
    const items = [
      item({ item_id: 'mine', agent_id: 'agent.a' }),
      item({ item_id: 'theirs', agent_id: 'agent.b' }),
      item({ item_id: 'noise', agent_id: 'agent.a', kind: 'notice' }),
      item({ item_id: 'seen', agent_id: 'agent.a', read_at: '2026-09-17T01:00:00Z' }),
    ];
    expect(unreadItemsFor(items, 'agent.a').map((i) => i.item_id)).toEqual(['mine']);
  });
});
