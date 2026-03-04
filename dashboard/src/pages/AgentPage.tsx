import { useEffect } from 'react';
import { useSearchParams } from 'react-router-dom';
import { useAgentContext } from '../contexts/AgentContext';
import { AgentTerminal } from '../components/AgentTerminal';
import { KernelMonitor } from '../components/KernelMonitor';

export function AgentPage() {
  const [searchParams] = useSearchParams();
  const {
    agents,
    selectedAgentId,
    setSelectedAgentId,
    systemActive,
    setSystemActive,
    refetchAgents,
  } = useAgentContext();

  // Sync URL params → Context
  useEffect(() => {
    const agentParam = searchParams.get('agent');
    const systemParam = searchParams.get('system');

    if (systemParam === 'true') {
      setSystemActive(true);
      setSelectedAgentId(null);
    } else if (agentParam) {
      setSelectedAgentId(agentParam);
      setSystemActive(false);
    } else {
      setSelectedAgentId(null);
      setSystemActive(false);
    }
  }, [searchParams, setSelectedAgentId, setSystemActive]);

  const selectedAgent = agents.find(a => a.id === selectedAgentId) || null;

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
