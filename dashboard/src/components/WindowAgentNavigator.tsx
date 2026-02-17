import React from 'react';
import {
  Cpu,
  Eye,
  MousePointer2,
  Settings,
  Plus,
  Zap,
  ShieldCheck
} from 'lucide-react';
import { AgentMetadata, Capability } from '../types';
import { AgentIcon, agentColor } from '../lib/agentIdentity';

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
    case 'VisionRead': return <Eye size={10} className="text-blue-500" />;
    case 'InputControl': return <MousePointer2 size={10} className="text-red-500" />;
    case 'MemoryRead':
    case 'MemoryWrite': return <Zap size={10} className="text-amber-500" />;
    default: return null;
  }
};

export const WindowAgentNavigator: React.FC<AgentNavigatorProps> = ({
  agents,
  activeAgentId,
  onSelectAgent,
  onSelectSystem,
  onAddAgent,
  systemActive
}) => {
  return (
    <div className="w-16 h-full flex flex-col items-center py-4 bg-slate-50/50 backdrop-blur-md border-r border-slate-200/60 gap-4">
      {/* System / Kernel Icon */}
      <button
        onClick={onSelectSystem}
        className={`relative group p-2.5 rounded-xl transition-all duration-300 ${
          systemActive 
            ? 'bg-white shadow-md text-[#2e4de6] ring-1 ring-[#2e4de6]/20' 
            : 'text-slate-400 hover:text-slate-600 hover:bg-white/60'
        }`}
        title="Exiv Kernel"
      >
        <Cpu size={20} />
        {systemActive && (
          <div className="absolute -left-1 top-1/2 -translate-y-1/2 w-1 h-5 bg-[#2e4de6] rounded-r-full" />
        )}
      </button>

      <div className="w-8 h-[1px] bg-slate-200/80" />

      {/* Agents List */}
      <div className="flex flex-col gap-3 overflow-y-auto no-scrollbar pb-2 w-full px-2 items-center">
        {agents.map((agent) => {
          const isActive = activeAgentId === agent.id && !systemActive;
          const accentColor = agentColor(agent);
          return (
            <div key={agent.id} className="relative flex flex-col items-center group w-full">
              <button
                onClick={() => onSelectAgent(agent.id)}
                className={`relative p-2.5 rounded-xl transition-all duration-300 overflow-hidden w-full flex justify-center ${
                  isActive
                    ? 'bg-white shadow-md'
                    : 'text-slate-400 hover:text-slate-600 hover:bg-white/60'
                }`}
                style={isActive ? { color: accentColor, boxShadow: `0 4px 6px -1px ${accentColor}20`, outline: `1px solid ${accentColor}20` } : undefined}
                title={agent.name}
              >
                <AgentIcon agent={agent} size={20} />

                {/* Status Indicator */}
                <div className={`absolute bottom-1.5 right-1.5 w-2 h-2 rounded-full border border-white ${
                  agent.status === 'online' ? 'bg-emerald-500' :
                  agent.status === 'degraded' ? 'bg-amber-500 animate-pulse' :
                  'bg-slate-300'
                }`} />
              </button>

              {/* Active Bar */}
              {isActive && (
                <div className="absolute -left-3 top-1/2 -translate-y-1/2 w-1 h-6 rounded-r-full" style={{ backgroundColor: accentColor }} />
              )}
            </div>
          );
        })}

        {/* Add Agent Button */}
        <button 
          onClick={onAddAgent}
          className="p-2.5 rounded-xl text-slate-300 hover:text-[#2e4de6] hover:bg-white/60 border border-slate-200 border-dashed transition-all duration-300 mt-2"
          title="Initialize New Agent"
        >
          <Plus size={20} />
        </button>
      </div>

      <div className="mt-auto flex flex-col gap-3">
        <button className="p-2.5 rounded-xl text-slate-400 hover:text-slate-600 hover:bg-white/60 transition-all">
          <ShieldCheck size={18} />
        </button>
        <button className="p-2.5 rounded-xl text-slate-400 hover:text-slate-600 hover:bg-white/60 transition-all">
          <Settings size={18} />
        </button>
      </div>
    </div>
  );
};
