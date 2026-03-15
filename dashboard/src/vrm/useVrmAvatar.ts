import { useEffect, useRef } from 'react';
import { useEventStream } from '../hooks/useEventStream';
import { EVENTS_URL } from '../services/api';
import { useVrmContext } from './VrmContext';

/**
 * Hook that subscribes to SSE events and drives avatar agent state transitions.
 * Maps: AgentThinking → thinking, ThoughtResponse → responding, AgenticLoopCompleted → idle.
 * For non-agentic cases, auto-transitions to idle 3s after ThoughtResponse.
 */
export function useVrmAvatar(agentId: string, apiKey?: string) {
  const { setAgentState, vrmEnabled } = useVrmContext();
  const idleTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEventStream(
    EVENTS_URL,
    (event) => {
      if (!vrmEnabled) return;
      if (!event.data || event.data.agent_id !== agentId) return;

      switch (event.type) {
        case 'AgentThinking':
          clearIdleTimeout();
          setAgentState('thinking');
          break;

        case 'ThoughtResponse':
          clearIdleTimeout();
          setAgentState('responding');
          // Non-agentic fallback: auto-idle after 3s if no AgenticLoopCompleted
          idleTimeoutRef.current = setTimeout(() => {
            setAgentState('idle');
          }, 3000);
          break;

        case 'AgenticLoopCompleted':
          clearIdleTimeout();
          setAgentState('idle');
          break;
      }
    },
    apiKey,
  );

  function clearIdleTimeout() {
    if (idleTimeoutRef.current) {
      clearTimeout(idleTimeoutRef.current);
      idleTimeoutRef.current = null;
    }
  }

  // Cleanup on unmount
  useEffect(() => {
    return () => clearIdleTimeout();
  }, [clearIdleTimeout]);
}
