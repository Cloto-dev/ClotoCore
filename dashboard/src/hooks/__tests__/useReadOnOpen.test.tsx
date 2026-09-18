import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const apiFns = vi.hoisted(() => ({
  apiKey: 'k',
  getNotifications: vi.fn(),
  markNotificationRead: vi.fn(),
}));
vi.mock('../useApi', () => ({ useApi: () => apiFns }));
vi.mock('../useEventStream', () => ({ useEventStream: () => {} }));

import { useReadOnOpen } from '../useUnreadAgents';

function item(over: Record<string, unknown>) {
  return {
    item_id: 'n',
    kind: 'approval',
    severity: 'warning',
    agent_id: 'agent.a',
    title: 'May I?',
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

beforeEach(() => {
  vi.clearAllMocks();
  apiFns.markNotificationRead.mockResolvedValue(undefined);
});

describe('opening a conversation', () => {
  it("reads that agent's waiting questions, and nobody else's", async () => {
    apiFns.getNotifications.mockResolvedValue([
      item({ item_id: 'q1' }),
      item({ item_id: 'q2', kind: 'proposal' }),
      item({ item_id: 'theirs', agent_id: 'agent.b' }),
      item({ item_id: 'rejected-tool', kind: 'notice' }),
      item({ item_id: 'seen', read_at: '2026-09-17T01:00:00Z' }),
    ]);

    renderHook(() => useReadOnOpen('agent.a'));

    await waitFor(() => expect(apiFns.markNotificationRead).toHaveBeenCalledTimes(2));
    expect(apiFns.markNotificationRead.mock.calls.map((c) => c[0]).sort()).toEqual(['q1', 'q2']);
  });

  it('reads nothing while no conversation is open', async () => {
    apiFns.getNotifications.mockResolvedValue([item({ item_id: 'q1' })]);

    renderHook(() => useReadOnOpen(null));
    await Promise.resolve();

    expect(apiFns.getNotifications).not.toHaveBeenCalled();
    expect(apiFns.markNotificationRead).not.toHaveBeenCalled();
  });

  it('reads again when a different agent is opened', async () => {
    apiFns.getNotifications.mockResolvedValue([
      item({ item_id: 'q1' }),
      item({ item_id: 'theirs', agent_id: 'agent.b' }),
    ]);

    const { rerender } = renderHook(({ id }: { id: string | null }) => useReadOnOpen(id), {
      initialProps: { id: 'agent.a' as string | null },
    });
    await waitFor(() => expect(apiFns.markNotificationRead).toHaveBeenCalledWith('q1'));

    rerender({ id: 'agent.b' });
    await waitFor(() => expect(apiFns.markNotificationRead).toHaveBeenCalledWith('theirs'));
    expect(apiFns.markNotificationRead).toHaveBeenCalledTimes(2);
  });
});
