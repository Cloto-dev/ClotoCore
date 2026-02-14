import React, { useState } from 'react';
import { AgentNavigator } from './AgentNavigator';
import { AgentTerminal } from './AgentTerminal'; // Previously SandboxCore
import { AgentMetadata } from '../types';

// Mock data for initial UI prototype
const MOCK_AGENTS: AgentMetadata[] = [
  {
    id: 'agent-karin',
    name: 'Karin',
    description: 'General Purpose AI Persona',
    capabilities: ['MemoryRead', 'MemoryWrite'],
    status: 'online'
  },
  {
    id: 'agent-color',
    name: 'Color',
    description: 'Universal Vision Framework',
    capabilities: ['VisionRead'],
    status: 'busy'
  },
  {
    id: 'agent-hand',
    name: 'Hand',
    description: 'Universal Action Framework',
    capabilities: ['InputControl', 'FileWrite'],
    status: 'online'
  }
];

export const ExivLayout: React.FC = () => {
  const [agents] = useState<AgentMetadata[]>(MOCK_AGENTS);
  const [activeAgentId, setActiveAgentId] = useState<string>(MOCK_AGENTS[0].id);
  const [systemActive, setSystemActive] = useState(false);

  const activeAgent = agents.find(a => a.id === activeAgentId);

  return (
    <div className="flex h-full w-full bg-[#050505] text-white overflow-hidden font-sans select-none">
      {/* Sidebar: Agent Navigator */}
      <AgentNavigator 
        agents={agents}
        activeAgentId={activeAgentId}
        onSelectAgent={(id) => {
          setActiveAgentId(id);
          setSystemActive(false);
        }}
        onSelectSystem={() => setSystemActive(true)}
        onAddAgent={() => console.log('Add agent clicked')}
        systemActive={systemActive}
      />

      {/* Main Content Area */}
      <main className="flex-1 relative flex flex-col overflow-hidden">
        {systemActive ? (
          /* Kernel / System View */
          <div className="flex-1 p-8 flex flex-col gap-6 animate-in fade-in slide-in-from-left-4 duration-500">
            <h1 className="text-3xl font-bold bg-clip-text text-transparent bg-gradient-to-r from-blue-400 to-cyan-300">
              Exiv Kernel v{__APP_VERSION__}
            </h1>
            <div className="grid grid-cols-3 gap-6">
              <div className="bg-white/5 border border-white/10 p-6 rounded-3xl backdrop-blur-md">
                <h3 className="text-white/60 text-sm mb-2">Total Tokens (Today)</h3>
                <div className="text-2xl font-mono">1,240,582</div>
              </div>
              <div className="bg-white/5 border border-white/10 p-6 rounded-3xl backdrop-blur-md">
                <h3 className="text-white/60 text-sm mb-2">Active Plugins</h3>
                <div className="text-2xl font-mono">12 / 15</div>
              </div>
              <div className="bg-white/5 border border-white/10 p-6 rounded-3xl backdrop-blur-md">
                <h3 className="text-white/60 text-sm mb-2">System Load</h3>
                <div className="text-2xl font-mono text-green-400">Stable</div>
              </div>
            </div>
            
            <div className="flex-1 bg-black/60 border border-white/5 rounded-3xl p-6 font-mono text-sm overflow-y-auto text-blue-300/80">
              <div>[SYSTEM] Kernel initialization complete.</div>
              <div>[SYSTEM] Communication adapter 'discord_gateway' started.</div>
              <div>[SYSTEM] Plugin 'plugin_google' loaded successfully.</div>
              <div>[SYSTEM] Agent 'Karin' is now online.</div>
              <div className="animate-pulse">_</div>
            </div>
          </div>
        ) : (
          /* Agent View */
          <div className="flex-1 flex flex-col overflow-hidden relative">
            {/* Header info for active agent */}
            <div className="absolute top-6 left-6 z-10 animate-in fade-in duration-700">
              <div className="flex items-center gap-3">
                <div className="px-3 py-1 bg-pink-500/20 border border-pink-500/30 rounded-full text-xs font-bold text-pink-400 uppercase tracking-widest shadow-[0_0_10px_rgba(236,72,153,0.2)]">
                  Active Agent
                </div>
                <h2 className="text-2xl font-bold">{activeAgent?.name}</h2>
              </div>
            </div>

            {/* Terminal / Chat Area */}
            <div className="flex-1 mt-16 p-6 overflow-hidden">
               <AgentTerminal />
            </div>

            {/* Right Panel Placeholder (for Vision/Hand logs later) */}
            <div className="absolute top-0 right-0 h-full w-80 border-l border-white/5 bg-white/[0.02] backdrop-blur-sm hidden xl:block p-6">
              <h3 className="text-xs font-bold text-white/40 uppercase tracking-widest mb-6">Capabilities & Context</h3>
              <div className="flex flex-col gap-6">
                <div className="p-4 bg-white/5 rounded-2xl border border-white/10">
                  <div className="text-xs text-white/60 mb-2">Description</div>
                  <div className="text-sm">{activeAgent?.description}</div>
                </div>
                {/* Vision/Hand live preview could go here */}
                <div className="flex-1 border-2 border-dashed border-white/5 rounded-3xl flex items-center justify-center text-white/10 italic text-sm text-center px-6">
                  Multimodal feedback (Color/Hand) will appear here
                </div>
              </div>
            </div>
          </div>
        )}
      </main>
    </div>
  );
};
