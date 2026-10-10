import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { NotificationItem } from '../services/api';
import { presenceState, usePartnerQuestions } from './usePartnerPresence';

const m = vi.hoisted(() => ({
  getNotifications: vi.fn(),
  apiKey: '',
  handlers: [] as Array<(event: { type: string; data: Record<string, unknown> }) => void>,
}));
vi.mock('../hooks/useApi', () => ({ useApi: () => m }));
vi.mock('../hooks/useEventStream', () => ({
  useEventStream: (_url: string, handler: (event: { type: string; data: Record<string, unknown> }) => void) => {
    m.handlers.push(handler);
  },
}));
const emit = (type: string, data: Record<string, unknown> = {}) =>
  act(() => {
    // Every render registers its handler; the newest one is the live one.
    m.handlers[m.handlers.length - 1]({ type, data });
  });

function item(over: Partial<NotificationItem>): NotificationItem {
  return {
    item_id: 'a1',
    kind: 'approval',
    severity: 'warning',
    agent_id: 'agent.mio',
    title: 'Run a command',
    body: null,
    created_at: '2026-10-10T00:00:00Z',
    read_at: null,
    resolved_at: null,
    decision: null,
    blocking: true,
    metadata: null,
    ...over,
  };
}

beforeEach(() => {
  vi.resetAllMocks();
  m.handlers.length = 0;
});

describe('presenceState', () => {
  const base = { connected: true, checking: false, asking: false, thinking: false };
  it('waits when nothing else is true', () => {
    expect(presenceState(base)).toBe('waiting');
  });
  it('reports a lost connection above a question and thinking', () => {
    expect(presenceState({ ...base, connected: false, asking: true, thinking: true })).toBe('offline');
  });
  it('does not report a lost connection before the first check answers', () => {
    expect(presenceState({ ...base, connected: false, checking: true })).toBe('waiting');
  });
  it('puts a waiting question above thinking', () => {
    expect(presenceState({ ...base, asking: true, thinking: true })).toBe('asking');
    expect(presenceState({ ...base, thinking: true })).toBe('thinking');
  });
});

describe('usePartnerQuestions', () => {
  it('counts only blocking, unresolved approvals held by this partner', async () => {
    m.getNotifications.mockResolvedValue([
      item({ item_id: 'mine' }),
      item({ item_id: 'other', agent_id: 'agent.other' }),
      item({ item_id: 'proposal', kind: 'proposal' }),
      item({ item_id: 'free', blocking: false }),
      item({ item_id: 'done', resolved_at: '2026-10-10T00:01:00Z' }),
      item({ item_id: 'mine2' }),
    ]);
    const { result } = renderHook(() => usePartnerQuestions('agent.mio'));
    await waitFor(() => expect(result.current).toBe(2));
    expect(m.getNotifications).toHaveBeenCalledWith(true);
  });

  it("adds this partner's new question from the stream and drops it when any window answers", async () => {
    m.getNotifications.mockResolvedValue([]);
    const { result } = renderHook(() => usePartnerQuestions('agent.mio'));
    await waitFor(() => expect(m.getNotifications).toHaveBeenCalled());
    emit('CommandApprovalRequested', { approval_id: 'x', agent_id: 'agent.other' });
    expect(result.current).toBe(0);
    emit('CommandApprovalRequested', { approval_id: 'q1', agent_id: 'agent.mio' });
    emit('CommandApprovalRequested', { approval_id: 'q2', agent_id: 'agent.mio' });
    expect(result.current).toBe(2);
    emit('CommandApprovalResult', { approval_id: 'x' });
    expect(result.current).toBe(2);
    emit('CommandApprovalResult', { approval_id: 'q1' });
    expect(result.current).toBe(1);
  });

  it('keeps the last count when a reload fails, and reloads after the stream reconnects', async () => {
    m.getNotifications.mockResolvedValueOnce([item({ item_id: 'q1' })]);
    const { result } = renderHook(() => usePartnerQuestions('agent.mio'));
    await waitFor(() => expect(result.current).toBe(1));
    m.getNotifications.mockRejectedValueOnce(new Error('down'));
    emit('__reconnected');
    await waitFor(() => expect(m.getNotifications).toHaveBeenCalledTimes(2));
    expect(result.current).toBe(1);
    m.getNotifications.mockResolvedValueOnce([]);
    emit('__lagged');
    await waitFor(() => expect(result.current).toBe(0));
  });

  it("never shows the previous partner's questions after switching", async () => {
    let release: (items: NotificationItem[]) => void = () => undefined;
    m.getNotifications.mockResolvedValueOnce([item({ item_id: 'q1' })]);
    const { result, rerender } = renderHook(({ id }) => usePartnerQuestions(id), { initialProps: { id: 'agent.mio' } });
    await waitFor(() => expect(result.current).toBe(1));
    m.getNotifications.mockReturnValueOnce(
      new Promise<NotificationItem[]>((resolve) => {
        release = resolve;
      }),
    );
    rerender({ id: 'agent.other' });
    expect(result.current).toBe(0);
    await act(async () => release([item({ item_id: 'q1' }), item({ item_id: 'o1', agent_id: 'agent.other' })]));
    expect(result.current).toBe(1);
  });
});
