import React from 'react';
import { 
  Cpu, 
  User, 
  Eye, 
  MousePointer2, 
  Settings, 
  Plus,
  Zap,
  ShieldCheck
} from 'lucide-react';
import { AgentMetadata, Capability } from '../types';

interface AgentNavigatorProps {
  agents: AgentMetadata[];
  activeAgentId?: string;
  onSelectAgent: (id: string) => void;
  onSelectSystem: () => void;
  onAddAgent: () => void;
  systemActive: boolean;
}

const CapabilityIcon = ({ capability }: { capability: Capability }) => {
  switch (capability) {
    case 'VisionRead': return <Eye size={10} className="text-blue-400" />;
    case 'InputControl': return <MousePointer2 size={10} className="text-red-400" />;
    case 'MemoryRead':
    case 'MemoryWrite': return <Zap size={10} className="text-yellow-400" />;
    default: return null;
  }
};

export const AgentNavigator: React.FC<AgentNavigatorProps> = ({
  agents,
  activeAgentId,
  onSelectAgent,
  onSelectSystem,
  onAddAgent,
  systemActive
}) => {
  return (
    <div className="w-20 h-full flex flex-col items-center py-6 bg-black/40 backdrop-blur-xl border-r border-white/10 gap-6">
      {/* System / Kernel Icon */}
      <button
        onClick={onSelectSystem}
        className={`relative group p-3 rounded-2xl transition-all duration-300 ${
          systemActive 
            ? 'bg-blue-500/20 text-blue-400 border border-blue-500/50 shadow-[0_0_15px_rgba(59,130,246,0.3)]' 
            : 'text-white/40 hover:text-white/70 hover:bg-white/5 border border-transparent'
        }`}
        title="Exiv Kernel"
      >
        <Cpu size={24} />
        {systemActive && (
          <div className="absolute -left-1 top-1/2 -translate-y-1/2 w-1 h-6 bg-blue-500 rounded-r-full shadow-[0_0_8px_rgba(59,130,246,0.8)]" />
        )}
      </button>

      <div className="w-10 h-[1px] bg-white/10" />

      {/* Agents List */}
      <div className="flex flex-col gap-4 overflow-y-auto no-scrollbar pb-4">
        {agents.map((agent) => {
          const isActive = activeAgentId === agent.id && !systemActive;
          return (
            <div key={agent.id} className="relative flex flex-col items-center group">
              <button
                onClick={() => onSelectAgent(agent.id)}
                className={`relative p-3 rounded-2xl transition-all duration-300 overflow-hidden ${
                  isActive
                    ? 'bg-pink-500/20 text-pink-400 border border-pink-500/50 shadow-[0_0_15px_rgba(236,72,153,0.3)]'
                    : 'text-white/60 hover:text-white hover:bg-white/10 border border-transparent'
                }`}
                title={agent.name}
              >
                <User size={24} />
                
                {/* Status Indicator */}
                <div className={`absolute bottom-1 right-1 w-2.5 h-2.5 rounded-full border-2 border-black ${
                  agent.status === 'online' ? 'bg-green-500 shadow-[0_0_5px_rgba(34,197,94,0.8)]' :
                  agent.status === 'busy' ? 'bg-blue-500 shadow-[0_0_5px_rgba(59,130,246,0.8)]' :
                  'bg-gray-500'
                }`} />
              </button>

              {/* Active Bar */}
              {isActive && (
                <div className="absolute -left-5 top-1/2 -translate-y-1/2 w-1 h-8 bg-pink-500 rounded-r-full shadow-[0_0_8px_rgba(236,72,153,0.8)]" />
              )}

              {/* Capability Badges (Horizontal tiny dots/icons under avatar) */}
              <div className="flex gap-1 mt-1">
                {agent.capabilities.slice(0, 3).map((cap, i) => (
                  <div key={i} title={cap} className="opacity-60 group-hover:opacity-100 transition-opacity">
                    <CapabilityIcon capability={cap} />
                  </div>
                ))}
              </div>
            </div>
          );
        })}

        {/* Add Agent Button */}
        <button 
          onClick={onAddAgent}
          className="p-3 rounded-2xl text-white/20 hover:text-white/60 hover:bg-white/5 border border-white/5 border-dashed transition-all duration-300"
          title="Initialize New Agent"
        >
          <Plus size={24} />
        </button>
      </div>

      <div className="mt-auto flex flex-col gap-4">
        <button className="p-3 rounded-2xl text-white/40 hover:text-white/80 hover:bg-white/5 transition-all">
          <ShieldCheck size={20} />
        </button>
        <button className="p-3 rounded-2xl text-white/40 hover:text-white/80 hover:bg-white/5 transition-all">
          <Settings size={20} />
        </button>
      </div>
    </div>
  );
};
