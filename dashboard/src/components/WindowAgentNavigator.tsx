import React from 'react';
import {
  Cpu,
  Plus,
} from 'lucide-react';
import { AgentMetadata } from '../types';
import { AgentIcon, agentColor } from '../lib/agentIdentity';

interface AgentNavigatorProps {
  agents: AgentMetadata[];
  activeAgentId?: string;
  onSelectAgent: (id: string) => void;
  onSelectSystem: () => void;
  onAddAgent: () => void;
  systemActive: boolean;
}

function statusLabel(status: string) {
  if (status === 'online') return 'Online';
  if (status === 'degraded') return 'Degraded';
  return 'Offline';
}

function statusDotClass(status: string) {
  if (status === 'online') return 'bg-emerald-500';
  if (status === 'degraded') return 'bg-amber-500 animate-pulse';
  return 'bg-content-muted';
}

export const WindowAgentNavigator: React.FC<AgentNavigatorProps> = ({
  agents,
  activeAgentId,
  onSelectAgent,
  onSelectSystem,
  onAddAgent,
  systemActive
}) => {
  return (
    <div className="w-48 h-full flex flex-col py-3 bg-surface-secondary/60 backdrop-blur-md border-r border-[var(--border-strong)]">
      {/* System / Kernel */}
      <button
        onClick={onSelectSystem}
        className={`relative mx-2 flex items-center gap-2.5 px-3 py-2 rounded-lg transition-all duration-200 ${
          systemActive
            ? 'bg-surface-primary shadow-sm text-brand ring-1 ring-brand/20'
            : 'text-content-tertiary hover:text-content-secondary hover:bg-glass-strong'
        }`}
      >
        {systemActive && (
          <div className="absolute left-0 top-1/2 -translate-y-1/2 w-1 h-5 bg-brand rounded-r-full" />
        )}
        <Cpu size={16} />
        <span className="text-[11px] font-bold tracking-wide uppercase">System</span>
      </button>

      <div className="mx-3 my-2 h-px bg-edge" />

      {/* Agents List */}
      <div className="flex-1 flex flex-col gap-1 overflow-y-auto no-scrollbar px-2">
        {agents.map((agent) => {
          const isActive = activeAgentId === agent.id && !systemActive;
          const accentColor = agentColor(agent);
          return (
            <button
              key={agent.id}
              onClick={() => onSelectAgent(agent.id)}
              className={`relative flex items-center gap-2.5 px-3 py-2 rounded-lg transition-all duration-200 text-left w-full ${
                isActive
                  ? 'bg-surface-primary shadow-sm'
                  : 'text-content-tertiary hover:text-content-secondary hover:bg-glass-strong'
              }`}
              style={isActive ? { color: accentColor, outline: `1px solid ${accentColor}20` } : undefined}
            >
              {isActive && (
                <div className="absolute left-0 top-1/2 -translate-y-1/2 w-1 h-5 rounded-r-full" style={{ backgroundColor: accentColor }} />
              )}
              <div className="relative flex-shrink-0">
                <AgentIcon agent={agent} size={16} />
                <div className={`absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full border-2 border-surface-secondary ${statusDotClass(agent.status)}`} />
              </div>
              <div className="min-w-0 flex-1">
                <div className="text-[11px] font-bold truncate">{agent.name}</div>
                <div className="text-[9px] text-content-muted font-mono truncate">{statusLabel(agent.status)}</div>
              </div>
            </button>
          );
        })}

        {/* Add Agent */}
        <button
          onClick={onAddAgent}
          className="flex items-center gap-2.5 px-3 py-2 rounded-lg text-content-muted hover:text-brand hover:bg-glass-strong border border-edge border-dashed transition-all duration-200 mt-1"
        >
          <Plus size={14} />
          <span className="text-[10px] font-bold uppercase tracking-wider">New Agent</span>
        </button>
      </div>
    </div>
  );
};
