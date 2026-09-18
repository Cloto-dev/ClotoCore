import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { PendingRejection } from '../../types';
import { ToolRejectionCard } from '../ToolRejectionCard';

// Echo i18n keys so the assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string, opts?: Record<string, unknown>) => (opts?.tool ? `${k}:${opts.tool}` : k),
  }),
}));

function build(overrides: Partial<PendingRejection> = {}): PendingRejection {
  return {
    agent_id: 'agent.test',
    engine_id: 'mind.local',
    tool_name: 'mgp.access.grant',
    call_id: 'call_1',
    code: 'YOLO_REQUIRED',
    reason: 'This tool is restricted to privileged (YOLO) mode, which is currently disabled.',
    remediation_hint: 'Ask the operator to enable YOLO mode in Settings → Security.',
    retryable: true,
    iteration: 1,
    local_id: 'call_1-rejection',
    received_at: Date.now(),
    ...overrides,
  };
}

describe('ToolRejectionCard', () => {
  it("says what was refused in the agent's words, why, and what can be done next", () => {
    render(<ToolRejectionCard rejection={build()} onDismiss={() => {}} />);
    expect(screen.getByText('ask.rejected:mgp.access.grant')).toBeInTheDocument();
    expect(screen.getByText(/privileged \(YOLO\) mode/i)).toBeInTheDocument();
    expect(screen.getByText(/Ask the operator to enable YOLO mode/)).toBeInTheDocument();
    expect(screen.getByText('YOLO_REQUIRED')).toBeInTheDocument();
  });

  it('carries nothing to answer: a refusal has no "enable it" shortcut', () => {
    render(<ToolRejectionCard rejection={build()} onDismiss={() => {}} />);
    const buttons = screen.getAllByRole('button');
    expect(buttons.map((b) => b.textContent)).toEqual(['ask.close']);
  });

  it('adds that a hard refusal cannot be changed from the settings, and only then', () => {
    const { rerender } = render(<ToolRejectionCard rejection={build({ retryable: false })} onDismiss={() => {}} />);
    expect(screen.getByText(/ask\.rejected_hard/)).toBeInTheDocument();
    rerender(<ToolRejectionCard rejection={build({ retryable: true })} onDismiss={() => {}} />);
    expect(screen.queryByText(/ask\.rejected_hard/)).not.toBeInTheDocument();
  });

  it('close takes the line away with its local id', () => {
    const onDismiss = vi.fn();
    render(<ToolRejectionCard rejection={build()} onDismiss={onDismiss} />);
    fireEvent.click(screen.getByRole('button', { name: 'ask.close' }));
    expect(onDismiss).toHaveBeenCalledWith('call_1-rejection');
  });
});
