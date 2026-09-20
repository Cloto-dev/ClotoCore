import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const apiFns = vi.hoisted(() => ({
  apiKey: 'k',
  getNotifications: vi.fn(),
  getUnreadAgents: vi.fn(),
  markNotificationRead: vi.fn(),
}));
vi.mock('../useApi', () => ({ useApi: () => apiFns }));
vi.mock('../useEventStream', () => ({ useEventStream: () => {} }));

import { useUnreadAgents } from '../useUnreadAgents';

function question(agentId: string) {
  return {
    item_id: `q-${agentId}`,
    kind: 'approval',
    severity: 'warning',
    agent_id: agentId,
    title: 'May I?',
    body: null,
    created_at: '2026-09-21T00:00:00Z',
    read_at: null,
    resolved_at: null,
    decision: null,
    blocking: true,
    metadata: null,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  apiFns.getNotifications.mockResolvedValue([]);
  apiFns.getUnreadAgents.mockResolvedValue([]);
});

describe('the roster marks', () => {
  it('marks an agent that is waiting on an answer, and one that merely spoke', async () => {
    // The two states are different and neither implies the other: an agent can
    // ask without speaking since, and speak without asking anything.
    apiFns.getNotifications.mockResolvedValue([question('agent.asked')]);
    apiFns.getUnreadAgents.mockResolvedValue(['agent.spoke']);

    const { result } = renderHook(() => useUnreadAgents());
    await waitFor(() => expect(result.current.unread.size).toBe(2));
    expect(result.current.unread.has('agent.asked')).toBe(true);
    expect(result.current.unread.has('agent.spoke')).toBe(true);
  });

  it('counts an agent once when it both asked and spoke', async () => {
    apiFns.getNotifications.mockResolvedValue([question('agent.a')]);
    apiFns.getUnreadAgents.mockResolvedValue(['agent.a']);

    const { result } = renderHook(() => useUnreadAgents());
    await waitFor(() => expect(result.current.unread.has('agent.a')).toBe(true));
    expect(result.current.unread.size).toBe(1);
  });

  it('keeps the source that answered when the other one fails', async () => {
    // A failed read is "I could not check", not "nothing is waiting". Losing
    // the half that did answer would turn one failure into a cleared mark.
    apiFns.getNotifications.mockRejectedValue(new Error('offline'));
    apiFns.getUnreadAgents.mockResolvedValue(['agent.spoke']);

    const { result } = renderHook(() => useUnreadAgents());
    await waitFor(() => expect(result.current.unread.has('agent.spoke')).toBe(true));
  });

  it('keeps the questions when the conversation source fails', async () => {
    apiFns.getNotifications.mockResolvedValue([question('agent.asked')]);
    apiFns.getUnreadAgents.mockRejectedValue(new Error('offline'));

    const { result } = renderHook(() => useUnreadAgents());
    await waitFor(() => expect(result.current.unread.has('agent.asked')).toBe(true));
  });

  it('holds the previous answer when both fail rather than claiming nothing waits', async () => {
    apiFns.getNotifications.mockResolvedValue([question('agent.a')]);
    apiFns.getUnreadAgents.mockResolvedValue(['agent.b']);
    const { result } = renderHook(() => useUnreadAgents());
    await waitFor(() => expect(result.current.unread.size).toBe(2));

    apiFns.getNotifications.mockRejectedValue(new Error('offline'));
    apiFns.getUnreadAgents.mockRejectedValue(new Error('offline'));
    // Inside act, so the state the refresh would set is flushed before the
    // assertion. Without it the assertion reads the value from before the
    // refresh and passes whatever the refresh did — measuring nothing.
    await act(async () => {
      await result.current.refreshUnread();
    });

    expect(result.current.unread.size).toBe(2);
  });

  it('asks both sources, so a mark can never come from one moment and one from another', async () => {
    renderHook(() => useUnreadAgents());
    await waitFor(() => {
      expect(apiFns.getNotifications).toHaveBeenCalled();
      expect(apiFns.getUnreadAgents).toHaveBeenCalled();
    });
  });
});
