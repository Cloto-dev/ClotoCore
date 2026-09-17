import { act, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { agentAccentTriplet, agentHue } from '../../lib/agentIdentity';

vi.mock('../../hooks/useAgents', () => ({
  useAgents: () => ({
    agents: [
      { id: 'agent.sapphy', name: 'Sapphy' },
      { id: 'agent.ks22', name: 'KS22' },
    ],
    isLoading: false,
    refetch: async () => {},
  }),
}));
vi.mock('../../hooks/useProcessingAgents', () => ({ useProcessingAgents: () => new Set<string>() }));

import { AgentProvider, useAgentContext } from '../AgentContext';

/**
 * The accent is the colour of the agent who is present (docs/DESIGN_PHILOSOPHY.md
 * §4.2). The helper that writes it is tested on its own; this pins that the
 * provider actually calls it when the selection changes — the part a screen
 * would never notice was missing, because the defaults still draw.
 */
afterEach(() => {
  document.documentElement.style.removeProperty('--h');
  document.documentElement.style.removeProperty('--agent');
});

describe('the agent provider', () => {
  it('gives the whole app the hue of the selected agent, and lets go when nobody is selected', () => {
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
    expect(root.getPropertyValue('--h')).toBe('');

    act(() => select?.('agent.ks22'));
    expect(root.getPropertyValue('--h')).toBe(String(agentHue({ id: 'agent.ks22' })));
    expect(root.getPropertyValue('--agent')).toBe(agentAccentTriplet({ id: 'agent.ks22' }));

    act(() => select?.('agent.sapphy'));
    expect(root.getPropertyValue('--h')).toBe(String(agentHue({ id: 'agent.sapphy' })));

    // An id that is not in the list is nobody.
    act(() => select?.('agent.gone'));
    expect(root.getPropertyValue('--h')).toBe('');

    act(() => select?.(null));
    expect(root.getPropertyValue('--agent')).toBe('');
  });
});
