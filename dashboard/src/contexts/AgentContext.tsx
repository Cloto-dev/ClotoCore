import { createContext, type ReactNode, useCallback, useContext, useEffect, useState } from 'react';
import { useAgents } from '../hooks/useAgents';
import { useProcessingAgents } from '../hooks/useProcessingAgents';
import { applyPresentAgent } from '../lib/agentIdentity';
import { THEME_APPLIED_EVENT } from '../themes/apply';
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

  // The selected agent is the one present: the accent takes its colour.
  // Keyed on the accent it resolves to, not on the agent object: a refetch
  // replaces every object, and a colour chosen in settings has to arrive here
  // without the selection changing.
  const presentAgent = agents.find((a) => a.id === selectedAgentId) ?? null;
  const presentAgentId = presentAgent?.id ?? null;
  const presentAccent = presentAgent?.metadata?.accent;
  useEffect(() => {
    const apply = () =>
      applyPresentAgent(
        presentAgentId === null ? null : { id: presentAgentId, metadata: { accent: presentAccent ?? '' } },
      );
    apply();
    // The accent is corrected against the surface on screen, and a theme may
    // hold the accent itself: a theme or mode change means writing it again.
    window.addEventListener(THEME_APPLIED_EVENT, apply);
    return () => window.removeEventListener(THEME_APPLIED_EVENT, apply);
  }, [presentAgentId, presentAccent]);

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
