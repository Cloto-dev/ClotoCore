import { afterEach, describe, expect, it } from 'vitest';
import type { Conversation } from '../../types';
import {
  displayTitle,
  groupConversations,
  groupOf,
  rememberConversation,
  rememberedConversation,
} from '../conversations';

function conv(id: string, updated_at: number, over: Partial<Conversation> = {}): Conversation {
  return {
    id,
    agent_id: 'agent.a',
    user_id: 'default',
    title: id,
    created_at: updated_at,
    updated_at,
    archived_at: null,
    message_count: 1,
    ...over,
  };
}

// A fixed "now": Wednesday 2026-09-16 15:00 local time.
const NOW = new Date(2026, 8, 16, 15, 0, 0).getTime();
const HOUR = 60 * 60 * 1000;
const DAY = 24 * HOUR;

afterEach(() => localStorage.clear());

describe('the sidebar groups', () => {
  it('are calendar days in local time, not rolling windows', () => {
    const midnightToday = new Date(2026, 8, 16, 0, 0, 0).getTime();
    expect(groupOf(NOW - HOUR, NOW)).toBe('today');
    expect(groupOf(midnightToday, NOW)).toBe('today');
    // 20 hours ago is inside the last 24 hours but on yesterday's calendar day.
    expect(groupOf(NOW - 20 * HOUR, NOW)).toBe('yesterday');
    expect(groupOf(midnightToday - 1, NOW)).toBe('yesterday');
    // 25 hours ago is outside the last 24 hours but still yesterday's calendar day.
    expect(groupOf(NOW - 25 * HOUR, NOW)).toBe('yesterday');
    expect(groupOf(midnightToday - DAY - 1, NOW)).toBe('previous_7_days');
    expect(groupOf(midnightToday - 7 * DAY, NOW)).toBe('previous_7_days');
    expect(groupOf(midnightToday - 7 * DAY - 1, NOW)).toBe('older');
  });

  it('order the list by activity, omit empty groups and never show an archived one', () => {
    const groups = groupConversations(
      [
        conv('old', NOW - 30 * DAY),
        conv('archived', NOW - HOUR, { archived_at: NOW }),
        conv('earlier-today', NOW - 5 * HOUR),
        conv('just-now', NOW - HOUR),
        conv('yesterday', NOW - 20 * HOUR),
      ],
      NOW,
    );
    expect(groups.map((g) => g.group)).toEqual(['today', 'yesterday', 'older']);
    expect(groups[0].items.map((c) => c.id)).toEqual(['just-now', 'earlier-today']);
    expect(groups.flatMap((g) => g.items).some((c) => c.id === 'archived')).toBe(false);
  });
});

describe('the open conversation', () => {
  it('is remembered per agent and forgotten on null', () => {
    expect(rememberedConversation('agent.a')).toBeNull();
    rememberConversation('agent.a', 'c1');
    rememberConversation('agent.b', 'c2');
    expect(rememberedConversation('agent.a')).toBe('c1');
    expect(rememberedConversation('agent.b')).toBe('c2');
    rememberConversation('agent.a', null);
    expect(rememberedConversation('agent.a')).toBeNull();
    expect(rememberedConversation('agent.b')).toBe('c2');
  });
});

describe('a row title', () => {
  it('falls back only when the kernel has not titled it', () => {
    expect(displayTitle({ title: '  ' }, 'New chat')).toBe('New chat');
    expect(displayTitle({ title: 'Plans' }, 'New chat')).toBe('Plans');
  });
});
