import { useAgentContext } from '../contexts/AgentContext';
import { AgentTerminal } from '../components/AgentTerminal';
import { KernelMonitor } from '../components/KernelMonitor';

export function AgentPage() {
  const {
    agents,
    selectedAgentId,
    setSelectedAgentId,
    systemActive,
    setSystemActive,
    refetchAgents,
  } = useAgentContext();

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
