import { useCallback, useRef, useState } from 'react';
import { EVENTS_URL } from '../services/api';
import { useApi } from './useApi';
import { useEventStream } from './useEventStream';

const TIMEOUT_MS = 60_000;

/**
 * Tracks which agents are currently processing (agentic loop active).
 * Returns a Set of agent IDs that are generating a response.
 */
export function useProcessingAgents(): Set<string> {
  const { apiKey } = useApi();
  const [processing, setProcessing] = useState<Set<string>>(new Set());
  const timers = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  const startProcessing = useCallback((agentId: string) => {
    // Clear existing timeout for this agent
    const existing = timers.current.get(agentId);
    if (existing) clearTimeout(existing);

    // Set 60s fallback timeout
    const timer = setTimeout(() => {
      timers.current.delete(agentId);
      setProcessing((prev) => {
        const next = new Set(prev);
        next.delete(agentId);
        return next;
      });
    }, TIMEOUT_MS);
    timers.current.set(agentId, timer);

    setProcessing((prev) => {
      if (prev.has(agentId)) return prev;
      const next = new Set(prev);
      next.add(agentId);
      return next;
    });
  }, []);

  const stopProcessing = useCallback((agentId: string) => {
    const existing = timers.current.get(agentId);
    if (existing) {
      clearTimeout(existing);
      timers.current.delete(agentId);
    }
    setProcessing((prev) => {
      if (!prev.has(agentId)) return prev;
      const next = new Set(prev);
      next.delete(agentId);
      return next;
    });
  }, []);

  const handleEvent = useCallback(
    (event: { type: string; data: Record<string, unknown> }) => {
      const agentId = (event.data as Record<string, unknown> | undefined)?.agent_id;
      if (typeof agentId !== 'string') {
        // ExternalAction uses target_agent_id
        const targetId = (event.data as Record<string, unknown> | undefined)?.target_agent_id;
        if (typeof targetId !== 'string') return;

        if (event.type === 'ExternalAction') {
          const status = (event.data as Record<string, unknown>)?.status;
          if (status === 'pending') {
            startProcessing(targetId);
          } else if (status === 'success' || status === 'error') {
            stopProcessing(targetId);
          }
        }
        return;
      }

      switch (event.type) {
        case 'MessageReceived':
        case 'AgentThinking':
          startProcessing(agentId);
          break;
        case 'ThoughtResponse':
          stopProcessing(agentId);
          break;
      }
    },
    [startProcessing, stopProcessing],
  );

  useEventStream(EVENTS_URL, handleEvent, apiKey);

  return processing;
}
