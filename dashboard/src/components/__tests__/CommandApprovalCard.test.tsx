import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CommandApprovalCard } from '../CommandApprovalCard';

const approveCommand = vi.fn().mockResolvedValue(undefined);
const trustCommand = vi.fn().mockResolvedValue(undefined);
const denyCommand = vi.fn().mockResolvedValue(undefined);

vi.mock('../../hooks/useApi', () => ({
  useApi: () => ({ approveCommand, trustCommand, denyCommand }),
}));

const COMMANDS = [{ command: 'rm -rf /tmp/scratch', command_name: 'rm' }];

describe('CommandApprovalCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('is still on screen and still answerable an hour after it appeared', () => {
    const onResolved = vi.fn();
    render(<CommandApprovalCard approvalId="a-1" commands={COMMANDS} onResolved={onResolved} />);

    // The kernel has no deadline on this question, so neither has the card. A
    // timer here would not merely show a stale number: the one it replaced
    // called `onResolved`, which is what takes the card off the screen — the
    // agent would have gone on waiting behind a question nobody could answer
    // any more.
    act(() => {
      vi.advanceTimersByTime(60 * 60 * 1000);
    });

    expect(onResolved).not.toHaveBeenCalled();
    expect(screen.getByText('rm -rf /tmp/scratch')).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText('Approve command'));
    expect(approveCommand).toHaveBeenCalledWith('a-1');
  });

  it('shows no countdown, because there is nothing left to count down to', () => {
    render(<CommandApprovalCard approvalId="a-2" commands={COMMANDS} onResolved={() => {}} />);

    expect(screen.queryByText(/^\d+s$/)).not.toBeInTheDocument();
  });

  it('answering resolves the card exactly once', async () => {
    const onResolved = vi.fn();
    render(<CommandApprovalCard approvalId="a-3" commands={COMMANDS} onResolved={onResolved} />);

    fireEvent.click(screen.getByLabelText('Deny command'));
    await act(async () => {
      await vi.runAllTimersAsync();
    });

    expect(denyCommand).toHaveBeenCalledWith('a-3');
    expect(onResolved).toHaveBeenCalledTimes(1);
    expect(onResolved).toHaveBeenCalledWith('a-3');
  });
});
