import { useEffect } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { useAgentContext } from '../contexts/AgentContext';
import { useConversations } from '../contexts/ConversationContext';
import { useApi } from '../hooks/useApi';
import { useEventStream } from '../hooks/useEventStream';
import { useMcpServers } from '../hooks/useMcpServers';
import { useReadOnOpen } from '../hooks/useUnreadAgents';
import { EVENTS_URL } from '../services/api';
import type { AgentMetadata } from '../types';
import { AgentConsole } from './AgentConsole';
import { AgentRoster } from './agents/AgentRoster';

export interface AgentTerminalProps {
  agents: AgentMetadata[];
  selectedAgent: AgentMetadata | null;
  onSelectAgent: (agent: AgentMetadata | null) => void;
  onRefresh: () => void;
  onBack?: () => void;
}

/**
 * The agent route, which is two screens: the roster when nobody is selected,
 * and the conversation when somebody is.
 *
 * Settings is neither — it is its own route (`/agents/:id/settings`), so it can
 * be linked to, reloaded, and left with the browser's Back.
 */
export function AgentTerminal({ agents, selectedAgent, onSelectAgent, onRefresh }: AgentTerminalProps) {
  const api = useApi();
  const navigate = useNavigate();
  const { processingAgentIds } = useAgentContext();
  const { refetch: refetchMcpServers } = useMcpServers();
  useReadOnOpen(selectedAgent?.id ?? null);

  // The agent view is persistently mounted — AppLayout hides it with CSS rather
  // than unmounting it — so useMcpServers' mount-time fetch never re-runs while
  // the app stays open, freezing the engine list at its startup snapshot.
  // Refetch whenever the agent route ('/') becomes active so an engine installed
  // elsewhere (Marketplace) appears in the engine picker without an app restart.
  const location = useLocation();
  useEffect(() => {
    if (location.pathname === '/') void refetchMcpServers();
  }, [location.pathname, refetchMcpServers]);

  // Listen for AgentPowerChanged events to auto-refresh
  useEventStream(
    EVENTS_URL,
    (event) => {
      if (event.type === 'AgentPowerChanged' || event.type === '__reconnected') {
        onRefresh();
      }
    },
    api.apiKey,
  );

  if (selectedAgent) {
    return (
      <OpenConversationConsole
        agent={selectedAgent}
        onBack={() => onSelectAgent(null)}
        onConfigure={() => navigate(`/agents/${encodeURIComponent(selectedAgent.id)}/settings`)}
      />
    );
  }

  return (
    <AgentRoster agents={agents} onSelectAgent={onSelectAgent} onRefresh={onRefresh} processing={processingAgentIds} />
  );
}

/** Mounts the console on the conversation that is open for the agent —
 * remembered, else newest, else new — and remounts it when that changes. */
function OpenConversationConsole({
  agent,
  onBack,
  onConfigure,
}: {
  agent: AgentMetadata;
  onBack: () => void;
  onConfigure: () => void;
}) {
  const { openFor, resolveOpen } = useConversations();
  const conversationId = openFor(agent.id);
  useEffect(() => {
    if (!conversationId) {
      resolveOpen(agent.id).catch((err) => {
        if (import.meta.env.DEV) console.error('Failed to open a conversation:', err);
      });
    }
  }, [agent.id, conversationId, resolveOpen]);
  if (!conversationId) return null;
  return (
    <AgentConsole
      key={`${agent.id}:${conversationId}`}
      agent={agent}
      conversationId={conversationId}
      onBack={onBack}
      onConfigure={onConfigure}
    />
  );
}
