import { fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { describe, expect, it, vi } from 'vitest';

// Echo i18n keys so assertions are deterministic without an i18n instance.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}));

import { applyRecallMetadata, normalizeRecallPolicy, normalizeSessionScope, RecallSection } from '../RecallSection';

describe('normalizeRecallPolicy', () => {
  it('passes through known values', () => {
    expect(normalizeRecallPolicy('manual_only')).toBe('manual_only');
    expect(normalizeRecallPolicy('session_start+active')).toBe('session_start+active');
  });
  it('falls back to always for unknown/absent', () => {
    expect(normalizeRecallPolicy(undefined)).toBe('always');
    expect(normalizeRecallPolicy('')).toBe('always');
    expect(normalizeRecallPolicy('bogus')).toBe('always');
  });
});

describe('normalizeSessionScope', () => {
  it('passes through known values', () => {
    expect(normalizeSessionScope('channel')).toBe('channel');
    expect(normalizeSessionScope('thread')).toBe('thread');
  });
  it('falls back to per_user for unknown/absent', () => {
    expect(normalizeSessionScope(undefined)).toBe('per_user');
    expect(normalizeSessionScope('nope')).toBe('per_user');
  });
});

describe('applyRecallMetadata', () => {
  it('writes non-default selections', () => {
    const m = applyRecallMetadata({}, 'manual_only', 'channel');
    expect(m).toEqual({ recall_policy: 'manual_only', session_scope: 'channel' });
  });

  it('omits both keys when both are default (absent == default)', () => {
    const m = applyRecallMetadata({ keep: '1' }, 'always', 'per_user');
    expect(m).toEqual({ keep: '1' });
    expect('recall_policy' in m).toBe(false);
    expect('session_scope' in m).toBe(false);
  });

  it('clears a previously-set key when reverted to default', () => {
    const m = applyRecallMetadata({ recall_policy: 'manual_only', session_scope: 'thread' }, 'always', 'per_user');
    expect(m).toEqual({});
  });

  it('writes one and clears the other independently', () => {
    const m = applyRecallMetadata({ session_scope: 'channel' }, 'session_start', 'per_user');
    expect(m).toEqual({ recall_policy: 'session_start' });
  });

  it('mutates and returns the same object reference', () => {
    const input: Record<string, string> = {};
    expect(applyRecallMetadata(input, 'always', 'per_user')).toBe(input);
  });
});

describe('RecallSection wiring', () => {
  function renderSection(overrides: Partial<ComponentProps<typeof RecallSection>> = {}) {
    const props: ComponentProps<typeof RecallSection> = {
      recallPolicy: 'always',
      sessionScope: 'per_user',
      precision: 'balanced',
      precisionSupported: true,
      onRecallPolicyChange: () => {},
      onSessionScopeChange: () => {},
      onPrecisionChange: () => {},
      ...overrides,
    };
    return render(<RecallSection {...props} />);
  }

  it('routes a policy pick to onRecallPolicyChange', () => {
    const onRecallPolicyChange = vi.fn();
    renderSection({ onRecallPolicyChange });
    // Open the timing pill (its label echoes the i18n key for "always").
    fireEvent.click(screen.getByRole('button', { name: /recall\.timing_always/ }));
    fireEvent.click(screen.getByText('recall.timing_manual'));
    expect(onRecallPolicyChange).toHaveBeenCalledExactlyOnceWith('manual_only');
  });

  it('routes a scope pick to onSessionScopeChange', () => {
    const onSessionScopeChange = vi.fn();
    renderSection({ onSessionScopeChange });
    fireEvent.click(screen.getByRole('button', { name: /recall\.scope_per_user/ }));
    fireEvent.click(screen.getByText('recall.scope_channel'));
    expect(onSessionScopeChange).toHaveBeenCalledExactlyOnceWith('channel');
  });

  it('routes a precision pick to onPrecisionChange when supported', () => {
    const onPrecisionChange = vi.fn();
    renderSection({ precisionSupported: true, onPrecisionChange });
    fireEvent.click(screen.getByRole('button', { name: /recall\.precision_balanced/ }));
    fireEvent.click(screen.getByText('recall.precision_strict'));
    expect(onPrecisionChange).toHaveBeenCalledExactlyOnceWith('strict');
  });

  it('disables the precision pill when the memory server does not support it', () => {
    const onPrecisionChange = vi.fn();
    renderSection({ precisionSupported: false, onPrecisionChange });
    const pill = screen.getByRole('button', { name: /recall\.precision_balanced/ });
    expect(pill).toBeDisabled();
    fireEvent.click(pill);
    expect(screen.queryByText('recall.precision_strict')).not.toBeInTheDocument();
    expect(onPrecisionChange).not.toHaveBeenCalled();
  });
});
