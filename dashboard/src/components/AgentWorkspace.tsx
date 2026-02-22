import { useState } from 'react';
import { AgentTerminal } from './AgentTerminal';
import { WindowAgentNavigator } from './WindowAgentNavigator';
import { KernelMonitor } from './KernelMonitor';
import { usePlugins } from '../hooks/usePlugins';
import { useAgents } from '../hooks/useAgents';

export function AgentWorkspace() {
  const { agents } = useAgents();
  const { plugins } = usePlugins();
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

  return (
    <div className="flex w-full h-full bg-transparent overflow-hidden">
      {/* Sidebar - Window Native Style */}
      <WindowAgentNavigator
        agents={agents}
        activeAgentId={selectedAgentId || undefined}
        onSelectAgent={handleSelectAgent}
        onSelectSystem={handleSelectSystem}
        onAddAgent={handleAddAgent}
        systemActive={systemActive}
      />

      {/* Main Content Area */}
      <div className="flex-1 h-full overflow-hidden relative">
         {systemActive ? (
           <KernelMonitor onClose={() => setSystemActive(false)} />
         ) : (
           <AgentTerminal
             agents={agents}
             plugins={plugins}
             selectedAgent={selectedAgent}
             onRefresh={fetchInitialData}
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
  );
}