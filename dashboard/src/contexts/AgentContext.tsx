import { createContext, useContext, useState, useCallback, type ReactNode } from 'react';
import { useAgents } from '../hooks/useAgents';
import type { AgentMetadata } from '../types';

interface AgentContextValue {
  agents: AgentMetadata[];
  isLoading: boolean;
  refetchAgents: () => Promise<void>;
  selectedAgentId: string | null;
  setSelectedAgentId: (id: string | null) => void;
  systemActive: boolean;
  setSystemActive: (active: boolean) => void;
}

const AgentContext = createContext<AgentContextValue | null>(null);

export function AgentProvider({ children }: { children: ReactNode }) {
  const { agents, isLoading, refetch } = useAgents();
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [systemActive, setSystemActive] = useState(false);

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
