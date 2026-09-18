import { act, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { agentAccentTriplet, setAccentSurface } from '../../lib/agentIdentity';
import { THEME_APPLIED_EVENT } from '../../themes/apply';

vi.mock('../../hooks/useAgents', () => ({
  useAgents: () => ({
    agents: [
      { id: 'agent.sapphy', name: 'Sapphy' },
      { id: 'agent.ks22', name: 'KS22' },
      { id: 'agent.painted', name: 'Painted', metadata: { accent: '300 60% 70%' } },
    ],
    isLoading: false,
    refetch: async () => {},
  }),
}));
vi.mock('../../hooks/useProcessingAgents', () => ({ useProcessingAgents: () => new Set<string>() }));

import { AgentProvider, useAgentContext } from '../AgentContext';

/**
 * The accent is the colour of the agent who is present (docs/DESIGN_PHILOSOPHY.md
 * §4.2); the surfaces do not follow the selection. The helper that writes the
 * accent is tested on its own; this pins that the provider actually calls it
 * when the selection changes — the part a screen would never notice was
 * missing, because the default still draws.
 */
afterEach(() => {
  document.documentElement.style.removeProperty('--h');
  document.documentElement.style.removeProperty('--agent');
  document.documentElement.style.removeProperty('--agent-ink');
  delete document.documentElement.dataset.accent;
  // The default theme's raised surface in dark (themes/packs/default.json).
  setAccentSurface([0.1488, 0.1675, 0.1712]);
});

describe('the agent provider', () => {
  it('gives the accent the colour of the selected agent, and lets go when nobody is selected', () => {
    let select: ((id: string | null) => void) | null = null;
    function Probe() {
      select = useAgentContext().setSelectedAgentId;
      return null;
    }
    render(
      <AgentProvider>
        <Probe />
      </AgentProvider>,
    );
    const root = document.documentElement.style;
    expect(root.getPropertyValue('--agent')).toBe('');

    act(() => select?.('agent.ks22'));
    expect(root.getPropertyValue('--agent')).toBe(agentAccentTriplet({ id: 'agent.ks22' }));
    // The surfaces keep their tint whoever is selected.
    expect(root.getPropertyValue('--h')).toBe('');

    act(() => select?.('agent.sapphy'));
    expect(root.getPropertyValue('--agent')).toBe(agentAccentTriplet({ id: 'agent.sapphy' }));

    // An id that is not in the list is nobody.
    act(() => select?.('agent.gone'));
    expect(root.getPropertyValue('--agent')).toBe('');

    act(() => select?.(null));
    expect(root.getPropertyValue('--agent')).toBe('');
  });

  it('uses the colour the agent was given in settings, not the one its id would give it', () => {
    let select: ((id: string | null) => void) | null = null;
    function Probe() {
      select = useAgentContext().setSelectedAgentId;
      return null;
    }
    render(
      <AgentProvider>
        <Probe />
      </AgentProvider>,
    );
    act(() => select?.('agent.painted'));
    const accent = document.documentElement.style.getPropertyValue('--agent');
    expect(accent).toBe('300 60% 70%');
    expect(accent).not.toBe(agentAccentTriplet({ id: 'agent.painted' }));
  });
  it('writes the accent again when a theme is applied, and lets go when the theme holds the accent', () => {
    let select: ((id: string | null) => void) | null = null;
    function Probe() {
      select = useAgentContext().setSelectedAgentId;
      return null;
    }
    render(
      <AgentProvider>
        <Probe />
      </AgentProvider>,
    );
    const root = document.documentElement;
    act(() => select?.('agent.ks22'));
    const onDark = root.style.getPropertyValue('--agent');

    // A light face: the same agent's accent has to be a different colour.
    act(() => {
      setAccentSurface([0.84, 0.85, 0.86]);
      window.dispatchEvent(new Event(THEME_APPLIED_EVENT));
    });
    expect(root.style.getPropertyValue('--agent')).toBe(agentAccentTriplet({ id: 'agent.ks22' }));
    expect(root.style.getPropertyValue('--agent')).not.toBe(onDark);

    // A theme with an accent of its own.
    act(() => {
      root.dataset.accent = 'fixed';
      window.dispatchEvent(new Event(THEME_APPLIED_EVENT));
    });
    expect(root.style.getPropertyValue('--agent')).toBe('');
  });
});
