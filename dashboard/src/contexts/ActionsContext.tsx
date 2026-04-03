import { createContext, type ReactNode, useCallback, useContext } from 'react';
import { type UseActionsResult, useActions } from '../hooks/useActions';
import { useApi } from '../hooks/useApi';
import { useEventStream } from '../hooks/useEventStream';
import { EVENTS_URL } from '../services/api';

const ActionsContext = createContext<UseActionsResult | null>(null);

export function ActionsProvider({ children }: { children: ReactNode }) {
  const actions = useActions();
  const { apiKey } = useApi();

  const handleEvent = useCallback(
    (event: { type: string; data?: Record<string, unknown> }) => {
      if (!event.data) return;

      if (event.type === 'AgentDialogue') {
        const d = event.data as {
          dialogue_id: string;
          caller_agent_id: string;
          caller_agent_name: string;
          target_agent_id: string;
          target_agent_name: string;
          prompt: string;
          engine_id: string;
          response: string | null;
          chain_depth: number;
          status: string;
        };
        actions.addOrUpdateDialogue({
          dialogue_id: d.dialogue_id,
          caller_agent_id: d.caller_agent_id,
          caller_agent_name: d.caller_agent_name,
          target_agent_id: d.target_agent_id,
          target_agent_name: d.target_agent_name,
          prompt: d.prompt,
          engine_id: d.engine_id,
          response: d.response,
          chain_depth: d.chain_depth,
          status: d.status as 'pending' | 'success' | 'error',
          timestamp: Date.now(),
        });
      }

      if (event.type === 'ExternalAction') {
        const d = event.data as {
          action_id: string;
          source: string;
          source_label: string;
          target_agent_id: string;
          target_agent_name: string;
          prompt: string;
          sender_name: string;
          engine_id: string;
          response: string | null;
          status: string;
          callback_id: string;
        };
        actions.addOrUpdateExternalAction({
          ...d,
          status: d.status as 'pending' | 'success' | 'error',
          timestamp: Date.now(),
        });
      }
    },
    [actions.addOrUpdateDialogue, actions.addOrUpdateExternalAction],
  );

  useEventStream(EVENTS_URL, handleEvent, apiKey);

  return <ActionsContext.Provider value={actions}>{children}</ActionsContext.Provider>;
}

export function useActionsContext(): UseActionsResult {
  const ctx = useContext(ActionsContext);
  if (!ctx) throw new Error('useActionsContext must be used within ActionsProvider');
  return ctx;
}
