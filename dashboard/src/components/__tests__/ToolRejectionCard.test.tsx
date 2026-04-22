import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { PendingRejection } from '../../types';
import { ToolRejectionCard } from '../ToolRejectionCard';

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
  it('renders code, tool name, reason, and remediation', () => {
    render(<ToolRejectionCard rejection={build()} onDismiss={() => {}} />);
    expect(screen.getByText('Tool Rejected')).toBeInTheDocument();
    expect(screen.getByText('YOLO_REQUIRED')).toBeInTheDocument();
    expect(screen.getByText('mgp.access.grant')).toBeInTheDocument();
    expect(screen.getByText(/privileged \(YOLO\) mode/i)).toBeInTheDocument();
    expect(screen.getByText(/Ask the operator to enable YOLO mode/)).toBeInTheDocument();
  });

  it('omits remediation block when hint is absent', () => {
    render(
      <ToolRejectionCard
        rejection={build({ remediation_hint: undefined, code: 'DELEGATION_CYCLE' })}
        onDismiss={() => {}}
      />,
    );
    expect(screen.queryByText(/Remediation/i)).not.toBeInTheDocument();
  });

  it('shows hard-rejection hint when retryable=false', () => {
    render(
      <ToolRejectionCard
        rejection={build({ retryable: false, code: 'DELEGATION_CYCLE', remediation_hint: undefined })}
        onDismiss={() => {}}
      />,
    );
    expect(screen.getByText(/Hard rejection — operator action cannot resolve this/)).toBeInTheDocument();
  });

  it('hides hard-rejection hint when retryable=true', () => {
    render(<ToolRejectionCard rejection={build({ retryable: true })} onDismiss={() => {}} />);
    expect(screen.queryByText(/Hard rejection/)).not.toBeInTheDocument();
  });

  it('fires onDismiss with local_id when X is clicked', () => {
    const onDismiss = vi.fn();
    render(<ToolRejectionCard rejection={build()} onDismiss={onDismiss} />);
    fireEvent.click(screen.getByLabelText('Dismiss rejection'));
    expect(onDismiss).toHaveBeenCalledWith('call_1-rejection');
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
