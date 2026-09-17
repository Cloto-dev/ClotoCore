import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const apiValue = vi.hoisted(() => ({ apiKey: 'k' }));
vi.mock('../useApi', () => ({ useApi: () => apiValue }));
const stream = vi.hoisted(() => ({
  handler: null as null | ((e: { type: string; data: Record<string, unknown> }) => void),
}));
vi.mock('../useEventStream', () => ({
  useEventStream: (_url: string, handler: (e: { type: string; data: Record<string, unknown> }) => void) => {
    stream.handler = handler;
  },
}));

import { useProcessingAgents } from '../useProcessingAgents';

function emit(type: string, data: Record<string, unknown>) {
  act(() => {
    stream.handler?.({ type, data });
  });
}

describe('which agents are working', () => {
  it('stops counting an agent when its reply arrives, and when its reply is stopped', () => {
    const { result } = renderHook(() => useProcessingAgents());
    emit('AgentThinking', { agent_id: 'agent.a' });
    emit('AgentThinking', { agent_id: 'agent.b' });
    expect([...result.current].sort()).toEqual(['agent.a', 'agent.b']);

    emit('ThoughtResponse', { agent_id: 'agent.a' });
    expect([...result.current]).toEqual(['agent.b']);

    // A stopped reply sends no ThoughtResponse: this is the only end there is.
    emit('ResponseStopped', { agent_id: 'agent.b', source_message_id: 'm1' });
    expect([...result.current]).toEqual([]);
  });
});
