import { createContext, type ReactNode, useCallback, useContext, useEffect, useState } from 'react';
import { useAgents } from '../hooks/useAgents';
import { useProcessingAgents } from '../hooks/useProcessingAgents';
import { applyPresentAgent } from '../lib/agentIdentity';
import type { AgentMetadata } from '../types';

interface AgentContextValue {
  agents: AgentMetadata[];
  isLoading: boolean;
  refetchAgents: () => Promise<void>;
  selectedAgentId: string | null;
  setSelectedAgentId: (id: string | null) => void;
  systemActive: boolean;
  setSystemActive: (active: boolean) => void;
  processingAgentIds: Set<string>;
}

const AgentContext = createContext<AgentContextValue | null>(null);

export function AgentProvider({ children }: { children: ReactNode }) {
  const { agents, isLoading, refetch } = useAgents();
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [systemActive, setSystemActive] = useState(false);
  const processingAgentIds = useProcessingAgents();

  // The selected agent is the one present: the whole app takes its hue.
  const presentAgentId = agents.some((a) => a.id === selectedAgentId) ? selectedAgentId : null;
  useEffect(() => {
    applyPresentAgent(presentAgentId === null ? null : { id: presentAgentId });
  }, [presentAgentId]);

  const refetchAgents = useCallback(async () => {
    await refetch();
  }, [refetch]);

  return (
    <AgentContext.Provider
      value={{
        agents,
        isLoading,
        refetchAgents,
        selectedAgentId,
        setSelectedAgentId,
        systemActive,
        setSystemActive,
        processingAgentIds,
      }}
    >
      {children}
    </AgentContext.Provider>
  );
}

export function useAgentContext() {
  const ctx = useContext(AgentContext);
  if (!ctx) throw new Error('useAgentContext must be used within AgentProvider');
  return ctx;
}
