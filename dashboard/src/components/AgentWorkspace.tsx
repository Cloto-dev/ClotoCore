import React, { useState, useEffect } from 'react';
import { AgentMetadata, PluginManifest } from '../types';
import { AgentTerminal } from './AgentTerminal';
import { WindowAgentNavigator } from './WindowAgentNavigator';
import { KernelMonitor } from './KernelMonitor';
import { AgentCreator } from './AgentCreator';

import { api } from '../services/api';

export function AgentWorkspace() {
  const [agents, setAgents] = useState<AgentMetadata[]>([]);
  const [plugins, setPlugins] = useState<PluginManifest[]>([]);
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [systemActive, setSystemActive] = useState(false);
  const [creatingAgent, setCreatingAgent] = useState(false);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    fetchInitialData();
  }, []);

  const fetchInitialData = async () => {
    try {
      const [agentsData, pluginsData] = await Promise.all([
        api.getAgents(),
        api.getPlugins()
      ]);
      setAgents(agentsData);
      setPlugins(pluginsData);
    } catch (err) {
      console.error('Failed to fetch initial data:', err);
    } finally {
      setIsLoading(false);
    }
  };

  const handleSelectAgent = (id: string) => {
    setSelectedAgentId(id);
    setSystemActive(false);
    setCreatingAgent(false);
  };

  const handleSelectSystem = () => {
    setSystemActive(!systemActive);
    setSelectedAgentId(null);
    setCreatingAgent(false);
  };

  const handleAddAgent = () => {
    setCreatingAgent(true);
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
         {creatingAgent ? (
           <AgentCreator onAgentCreated={() => { setCreatingAgent(false); fetchInitialData(); }} />
         ) : systemActive ? (
           <KernelMonitor onClose={() => setSystemActive(false)} />
         ) : (
           <AgentTerminal 
             agents={agents}
             plugins={plugins}
             selectedAgent={selectedAgent}
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