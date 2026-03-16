import { useEffect } from 'react';
import { useLocation, useSearchParams } from 'react-router-dom';
import { AgentTerminal } from '../components/AgentTerminal';
import { KernelMonitor } from '../components/KernelMonitor';
import { useAgentContext } from '../contexts/AgentContext';

export function AgentPage() {
  const [searchParams] = useSearchParams();
  const location = useLocation();
  const { agents, selectedAgentId, setSelectedAgentId, systemActive, setSystemActive, refetchAgents } =
    useAgentContext();

  // Sync URL params → Context (only when agent route is active)
  // When hidden (navigated to /mcp-servers etc.), searchParams returns empty
  // which would reset selectedAgentId — skip sync to preserve state.
  useEffect(() => {
    if (location.pathname !== '/') return;

    const agentParam = searchParams.get('agent');
    const systemParam = searchParams.get('system');

    if (systemParam === 'true') {
      setSystemActive(true);
      setSelectedAgentId(null);
    } else if (agentParam) {
      setSelectedAgentId(agentParam);
      setSystemActive(false);
    }
  }, [searchParams, location.pathname, setSelectedAgentId, setSystemActive]);

  const selectedAgent = agents.find((a) => a.id === selectedAgentId) || null;

  if (systemActive) {
    return <KernelMonitor onClose={() => setSystemActive(false)} />;
  }

  return (
    <AgentTerminal
      agents={agents}
      selectedAgent={selectedAgent}
      onRefresh={refetchAgents}
      onSelectAgent={(agent) => {
        if (agent) {
          setSelectedAgentId(agent.id);
          setSystemActive(false);
        } else {
          setSelectedAgentId(null);
          setSystemActive(false);
        }
      }}
    />
  );
}
