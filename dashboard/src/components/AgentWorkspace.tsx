import { useState } from 'react';
import { Users } from 'lucide-react';
import { AgentTerminal } from './AgentTerminal';
import { ViewHeader } from './ViewHeader';
import { WindowAgentNavigator } from './WindowAgentNavigator';
import { InteractiveGrid } from './InteractiveGrid';
import { KernelMonitor } from './KernelMonitor';
import { useAgents } from '../hooks/useAgents';

export function AgentWorkspace({ onBack }: { onBack?: () => void }) {
  const { agents, refetch: refetchAgents } = useAgents();

  const fetchInitialData = () => {
    refetchAgents();
  };
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [systemActive, setSystemActive] = useState(false);

  const handleSelectAgent = (id: string) => {
    setSelectedAgentId(id);
    setSystemActive(false);
  };

  const handleSelectSystem = () => {
    setSystemActive(!systemActive);
    setSelectedAgentId(null);
  };

  const handleAddAgent = () => {
    // Deselect current agent to show the management view (which includes the creation form)
    setSelectedAgentId(null);
    setSystemActive(false);
  };

  const selectedAgent = agents.find(a => a.id === selectedAgentId) || null;

  const activeCount = agents.filter(a => a.enabled).length;

  return (
    <div className="flex flex-col w-full h-full bg-surface-base overflow-hidden relative">
      {/* Full-width header */}
      <ViewHeader
        icon={Users}
        title="Agent Hub"
        onBack={onBack}
        right={<span className="text-[10px] font-mono text-content-tertiary">{activeCount} / {agents.length} Active</span>}
      />

      {/* Body: sidebar + content */}
      <div className="flex flex-1 overflow-hidden relative">
        <InteractiveGrid />

        {/* Sidebar - Window Native Style */}
        <div className="relative z-10">
          <WindowAgentNavigator
          agents={agents}
          activeAgentId={selectedAgentId || undefined}
          onSelectAgent={handleSelectAgent}
          onSelectSystem={handleSelectSystem}
          onAddAgent={handleAddAgent}
          systemActive={systemActive}
        />
      </div>

      {/* Main Content Area */}
      <div className="flex-1 h-full overflow-hidden relative z-10">
         {systemActive ? (
           <KernelMonitor onClose={() => setSystemActive(false)} />
         ) : (
           <AgentTerminal
             agents={agents}
             selectedAgent={selectedAgent}
             onRefresh={fetchInitialData}
             onBack={onBack}
             onSelectAgent={(agent) => {
               if (agent) {
                 handleSelectAgent(agent.id);
               } else {
                 setSelectedAgentId(null);
                 setSystemActive(false);
               }
             }}
           />
         )}
      </div>
      </div>
    </div>
  );
}