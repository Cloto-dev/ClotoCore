import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CommandApprovalCard } from '../CommandApprovalCard';

const approveCommand = vi.fn().mockResolvedValue(undefined);
const trustCommand = vi.fn().mockResolvedValue(undefined);
const denyCommand = vi.fn().mockResolvedValue(undefined);

vi.mock('../../hooks/useApi', () => ({
  useApi: () => ({ approveCommand, trustCommand, denyCommand }),
}));
// Echo i18n keys so the assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
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

    fireEvent.click(screen.getByRole('button', { name: 'ask.always_allow_this' }));
    expect(approveCommand).toHaveBeenCalledWith('a-1');
  });

  it('shows no countdown, because there is nothing left to count down to', () => {
    render(<CommandApprovalCard approvalId="a-2" commands={COMMANDS} onResolved={() => {}} />);

    expect(screen.queryByText(/^\d+s$/)).not.toBeInTheDocument();
  });

  // Each label promises what the kernel keeps: trust holds the program name in
  // memory until restart, approve saves the exact command for good. Swapping
  // the two calls would make each button do what the other one says.
  it("'allow until restart' trusts the program name; it does not save the command", () => {
    render(<CommandApprovalCard approvalId="a-4" commands={COMMANDS} onResolved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: 'ask.allow_until_restart' }));
    expect(trustCommand).toHaveBeenCalledWith('a-4');
    expect(approveCommand).not.toHaveBeenCalled();
  });

  it("'always allow this command' saves the exact command; it does not trust the program name", () => {
    render(<CommandApprovalCard approvalId="a-7" commands={COMMANDS} onResolved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: 'ask.always_allow_this' }));
    expect(approveCommand).toHaveBeenCalledWith('a-7');
    expect(trustCommand).not.toHaveBeenCalled();
  });

  it('names several commands in the plural, and still saves them rather than trusting their programs', () => {
    const several = [...COMMANDS, { command: 'ls /tmp', command_name: 'ls' }];
    render(<CommandApprovalCard approvalId="a-8" commands={several} onResolved={() => {}} />);
    expect(screen.queryByRole('button', { name: 'ask.always_allow_this' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'ask.always_allow_these' }));
    expect(approveCommand).toHaveBeenCalledWith('a-8');
    expect(trustCommand).not.toHaveBeenCalled();
  });

  it('names the impact the kernel derived, and nothing when it did not', () => {
    const { rerender } = render(
      <CommandApprovalCard approvalId="a-5" commands={COMMANDS} severity="error" onResolved={() => {}} />,
    );
    expect(screen.getByText('ask.risk')).toBeInTheDocument();
    rerender(<CommandApprovalCard approvalId="a-6" commands={COMMANDS} onResolved={() => {}} />);
    expect(screen.queryByText('ask.risk')).not.toBeInTheDocument();
  });

  it('answering resolves the card exactly once', async () => {
    const onResolved = vi.fn();
    render(<CommandApprovalCard approvalId="a-3" commands={COMMANDS} onResolved={onResolved} />);

    fireEvent.click(screen.getByRole('button', { name: 'ask.not_now' }));
    await act(async () => {
      await vi.runAllTimersAsync();
    });

    expect(denyCommand).toHaveBeenCalledWith('a-3');
    expect(onResolved).toHaveBeenCalledTimes(1);
    expect(onResolved).toHaveBeenCalledWith('a-3');
  });
});
