import { act, renderHook, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { useAgents } from '../useAgents';

const m = vi.hoisted(() => ({
  ready: Promise.resolve(true),
  api: { apiKey: 'session-order-test', getAgents: vi.fn() },
}));
vi.mock('../useApi', () => ({ useApi: () => m.api }));
vi.mock('../../services/session', () => ({ browserSessionReady: () => m.ready }));
vi.mock('../../mate/usePartnerMedia', () => ({ PARTNER_SETTINGS_CHANGED: 'mizmate-partner-settings-changed' }));

it('waits for the session before fetching partners and refreshes after shared settings change', async () => {
  let grant!: (value: boolean) => void;
  m.ready = new Promise<boolean>((resolve) => {
    grant = resolve;
  });
  m.api.getAgents.mockResolvedValue([{ id: 'agent.mio', name: 'Mio', metadata: {} }]);
  const { result } = renderHook(() => useAgents());
  expect(m.api.getAgents).not.toHaveBeenCalled();
  await act(async () => grant(true));
  await waitFor(() => expect(result.current.agents[0]?.name).toBe('Mio'));
  m.api.getAgents.mockResolvedValue([{ id: 'agent.mio', name: 'Mio updated', metadata: {} }]);
  act(() => window.dispatchEvent(new CustomEvent('mizmate-partner-settings-changed', { detail: 'agent.mio' })));
  await waitFor(() => expect(result.current.agents[0]?.name).toBe('Mio updated'));
  expect(m.api.getAgents).toHaveBeenCalledTimes(2);
});
