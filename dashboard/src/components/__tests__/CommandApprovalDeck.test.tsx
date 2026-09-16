import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { getNotifications, approveCommand, trustCommand, denyCommand } = vi.hoisted(() => ({
  getNotifications: vi.fn(),
  approveCommand: vi.fn(),
  trustCommand: vi.fn(),
  denyCommand: vi.fn(),
}));
vi.mock('../../hooks/useApi', () => ({
  useApi: () => ({ getNotifications, approveCommand, trustCommand, denyCommand, apiKey: 'k' }),
}));

// Hold the stream handler so a test can deliver events the way the kernel does.
const { streamHandler } = vi.hoisted(() => ({ streamHandler: { current: null as ((e: unknown) => void) | null } }));
vi.mock('../../hooks/useEventStream', () => ({
  useEventStream: (_url: string, onMessage: (e: unknown) => void) => {
    streamHandler.current = onMessage;
  },
}));

import { CommandApprovalDeck, deckReducer, PEEK_MS, RAISE_APPROVAL_EVENT, REARM_MS } from '../CommandApprovalDeck';

function requested(id: string, command: string, severity = 'error') {
  return {
    type: 'CommandApprovalRequested',
    data: {
      approval_id: id,
      agent_id: `agent.${id}`,
      commands: [{ call_id: `c-${id}`, command, command_name: command.split(' ')[0], severity }],
    },
  };
}

function deliver(event: unknown) {
  act(() => {
    streamHandler.current?.(event);
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  streamHandler.current = null;
  getNotifications.mockResolvedValue([]);
  approveCommand.mockResolvedValue(undefined);
  trustCommand.mockResolvedValue(undefined);
  denyCommand.mockResolvedValue(undefined);
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  localStorage.clear();
});

describe('CommandApprovalDeck', () => {
  it('shows nothing until something asks', () => {
    render(<CommandApprovalDeck />);
    expect(screen.queryByTestId('approval-deck')).not.toBeInTheDocument();
  });

  it('raises a card for an arriving request, naming the agent that asked', () => {
    render(<CommandApprovalDeck />);
    deliver(requested('a1', 'rm -rf /tmp/scratch'));

    expect(screen.getByTestId('approval-deck')).toBeInTheDocument();
    expect(screen.getByText('rm -rf /tmp/scratch')).toBeInTheDocument();
    // At the window level nothing else says whose question this is.
    expect(screen.getByTestId('approval-agent')).toHaveTextContent('agent.a1');
  });

  it('stops showing the card after the peek window without resolving anything', () => {
    render(<CommandApprovalDeck />);
    deliver(requested('a1', 'rm -rf /tmp/scratch'));

    act(() => {
      vi.advanceTimersByTime(PEEK_MS + 1000);
    });

    expect(screen.queryByTestId('approval-deck')).not.toBeInTheDocument();
    // The whole point of a short window: nothing was answered on the reader's
    // behalf. An expiry that denied would be the deadline under another name.
    expect(approveCommand).not.toHaveBeenCalled();
    expect(denyCommand).not.toHaveBeenCalled();
    expect(trustCommand).not.toHaveBeenCalled();

    // And the item is still there to come back to — this is what the bell does.
    act(() => {
      window.dispatchEvent(new CustomEvent(RAISE_APPROVAL_EVENT, { detail: { approvalId: 'a1' } }));
    });
    expect(screen.getByText('rm -rf /tmp/scratch')).toBeInTheDocument();
  });

  it('paging restarts the peek window, because paging is reading', () => {
    render(<CommandApprovalDeck />);
    deliver(requested('a1', 'echo one'));
    deliver(requested('a2', 'echo two'));

    act(() => {
      vi.advanceTimersByTime(PEEK_MS - 2000);
    });
    expect(screen.getByTestId('approval-deck')).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText('Next request'));

    // Past the original deadline, and still up because the reader was reading.
    act(() => {
      vi.advanceTimersByTime(4000);
    });
    expect(screen.getByTestId('approval-deck')).toBeInTheDocument();
    expect(screen.getByTestId('approval-pager')).toHaveTextContent('2 / 2');
  });

  it('a new arrival does not change what the reader is looking at', () => {
    render(<CommandApprovalDeck />);
    deliver(requested('a1', 'echo one'));
    deliver(requested('a2', 'echo two'));
    fireEvent.click(screen.getByLabelText('Next request'));
    expect(screen.getByText('echo two')).toBeInTheDocument();

    deliver(requested('a3', 'echo three'));

    // Same page, same content; the arrival announces itself in the count only.
    expect(screen.getByText('echo two')).toBeInTheDocument();
    expect(screen.queryByText('echo three')).not.toBeInTheDocument();
    expect(screen.getByTestId('approval-pager')).toHaveTextContent('2 / 3');
  });

  it('holds the buttons briefly after an answer, so a second click cannot carry over', async () => {
    render(<CommandApprovalDeck />);
    deliver(requested('a1', 'echo one'));
    deliver(requested('a2', 'echo two'));

    fireEvent.click(screen.getByLabelText('Approve command'));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(approveCommand).toHaveBeenCalledWith('a1');
    // The next request has taken the same coordinates on screen.
    expect(screen.getByText('echo two')).toBeInTheDocument();
    expect(screen.getByLabelText('Approve command')).toBeDisabled();

    fireEvent.click(screen.getByLabelText('Approve command'));
    expect(approveCommand).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(REARM_MS + 50);
    });
    expect(screen.getByLabelText('Approve command')).toBeEnabled();
  });

  it('a request settled anywhere else leaves the deck', () => {
    render(<CommandApprovalDeck />);
    deliver(requested('a1', 'echo one'));
    expect(screen.getByTestId('approval-deck')).toBeInTheDocument();

    deliver({ type: 'CommandApprovalResult', data: { approval_id: 'a1', decision: 'denied by user' } });

    expect(screen.queryByTestId('approval-deck')).not.toBeInTheDocument();
  });

  it('a request quieter than the threshold waits instead of interrupting', () => {
    localStorage.setItem('cloto-notification-threshold', 'high');
    render(<CommandApprovalDeck />);
    deliver(requested('a1', 'ls /tmp', 'warning'));

    // Not raised — but not discarded either: the bell can still call it up, and
    // the badge (which never consults the threshold) still counts it.
    expect(screen.queryByTestId('approval-deck')).not.toBeInTheDocument();
    act(() => {
      window.dispatchEvent(new CustomEvent(RAISE_APPROVAL_EVENT, { detail: { approvalId: 'a1' } }));
    });
    expect(screen.getByText('ls /tmp')).toBeInTheDocument();
  });

  it('picks up what was already waiting before the window opened, without interrupting', async () => {
    getNotifications.mockResolvedValue([
      {
        item_id: 'old-1',
        kind: 'approval',
        severity: 'error',
        agent_id: 'agent.growth',
        title: '1 command(s) awaiting approval',
        body: 'curl example.com',
        created_at: '2026-09-16T00:00:00Z',
        read_at: null,
        resolved_at: null,
        decision: null,
        blocking: true,
        metadata: { commands: [{ command: 'curl example.com', command_name: 'curl', severity: 'error' }] },
      },
    ]);
    render(<CommandApprovalDeck />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(getNotifications).toHaveBeenCalledWith(true);

    // A reload must not strand a blocked agent. It also must not throw a card up
    // on every page load for something that has been waiting all along.
    expect(screen.queryByTestId('approval-deck')).not.toBeInTheDocument();
    act(() => {
      window.dispatchEvent(new CustomEvent(RAISE_APPROVAL_EVENT, { detail: { approvalId: 'old-1' } }));
    });
    expect(screen.getByText('curl example.com')).toBeInTheDocument();
    expect(screen.getByTestId('approval-agent')).toHaveTextContent('agent.growth');
  });
});

// The rules about position are easier to pin on the reducer than through the
// DOM, and they are the rules most likely to be broken by a later refactor.
describe('deckReducer', () => {
  const entry = (id: string) => ({ approvalId: id, agentId: 'a', commands: [], severity: 'error' as const });
  const base = { queue: [entry('x'), entry('y'), entry('z')], index: 1, open: true, peek: 0 };

  it('resolving something ahead of the cursor keeps the reader on the same request', () => {
    const next = deckReducer(base, { type: 'resolved', approvalId: 'x' });
    expect(next.queue.map((a) => a.approvalId)).toEqual(['y', 'z']);
    expect(next.queue[next.index].approvalId).toBe('y');
  });

  it('resolving something after the cursor leaves the cursor alone', () => {
    const next = deckReducer(base, { type: 'resolved', approvalId: 'z' });
    expect(next.queue[next.index].approvalId).toBe('y');
  });

  it('resolving the last one closes the deck', () => {
    const one = { queue: [entry('x')], index: 0, open: true, peek: 0 };
    expect(deckReducer(one, { type: 'resolved', approvalId: 'x' }).open).toBe(false);
  });

  it('expiring closes the card and touches nothing else', () => {
    const next = deckReducer(base, { type: 'expire' });
    expect(next.open).toBe(false);
    expect(next.queue).toEqual(base.queue);
    expect(next.index).toBe(base.index);
  });

  it('paging wraps in both directions', () => {
    expect(deckReducer({ ...base, index: 2 }, { type: 'page', delta: 1 }).index).toBe(0);
    expect(deckReducer({ ...base, index: 0 }, { type: 'page', delta: -1 }).index).toBe(2);
  });

  it('the same request arriving twice is one request', () => {
    const next = deckReducer(base, { type: 'requested', entry: entry('y'), raises: true });
    expect(next.queue).toHaveLength(3);
  });
});
