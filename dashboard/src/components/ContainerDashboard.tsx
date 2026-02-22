import { Puzzle, Cpu, Terminal, Lock } from 'lucide-react';
import { AgentMetadata, PluginManifest } from '../types';
import { AgentIcon, agentColor, statusBadgeClass, statusDotColor } from '../lib/agentIdentity';
import { AgentPowerButton } from './AgentPowerButton';

export function ContainerDashboard({ agent, plugins, onBack, onConfigure, onPowerToggle }: {
  agent: AgentMetadata;
  plugins: PluginManifest[];
  onBack: () => void;
  onConfigure: () => void;
  onPowerToggle: (agent: AgentMetadata) => void;
}) {
  const color = agentColor(agent);
  const enginePlugin = plugins.find(p => p.id === agent.default_engine_id);
  const memoryPlugin = plugins.find(p => p.id === agent.metadata?.preferred_memory);

  return (
    <div className="flex flex-col h-full bg-glass backdrop-blur-3xl animate-in fade-in duration-500">
      {/* Header */}
      <div className="p-4 border-b border-edge-subtle flex items-center justify-between bg-glass-strong">
        <div className="flex items-center gap-3">
          <div className="p-2 text-white rounded-lg shadow-lg" style={{ backgroundColor: color, boxShadow: `0 10px 15px -3px ${color}33` }}>
            <AgentIcon agent={agent} size={18} />
          </div>
          <div>
            <h2 className="text-sm font-black text-content-primary tracking-tight uppercase">{agent.name}</h2>
            <div className="flex items-center gap-2">
              <span className={`w-1.5 h-1.5 rounded-full ${statusDotColor(agent.status)}`} />
              <span className="text-[8px] font-mono text-content-tertiary uppercase tracking-widest">
                Container Process {agent.enabled ? '· Running' : '· Stopped'}
              </span>
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <AgentPowerButton agent={agent} onPowerToggle={onPowerToggle} />
          <button
            onClick={onBack}
            className="px-4 py-1.5 rounded-full border border-edge text-[9px] font-bold text-content-tertiary hover:text-content-secondary hover:border-edge transition-all uppercase tracking-widest"
          >
            Back
          </button>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-8 no-scrollbar">
        <div className="max-w-lg mx-auto space-y-6">
          {/* Description */}
          <div className="bg-surface-primary border border-edge-subtle rounded-2xl p-6 shadow-sm">
            <h3 className="text-[10px] font-black text-content-tertiary uppercase tracking-[0.15em] mb-3">Description</h3>
            <p className="text-sm text-content-secondary leading-relaxed">
              {agent.description || 'No description provided.'}
            </p>
          </div>

          {/* Configuration */}
          <div className="bg-surface-primary border border-edge-subtle rounded-2xl p-6 shadow-sm">
            <h3 className="text-[10px] font-black text-content-tertiary uppercase tracking-[0.15em] mb-4">Configuration</h3>
            <div className="space-y-3">
              <div className="flex items-center justify-between py-2 border-b border-edge-subtle">
                <span className="text-[11px] font-bold text-content-secondary">Agent ID</span>
                <span className="text-[11px] font-mono text-content-tertiary">{agent.id}</span>
              </div>
              <div className="flex items-center justify-between py-2 border-b border-edge-subtle">
                <span className="text-[11px] font-bold text-content-secondary">Bridge Engine</span>
                <span className="text-[11px] font-mono text-content-tertiary">{enginePlugin?.name || agent.default_engine_id || 'None'}</span>
              </div>
              <div className="flex items-center justify-between py-2 border-b border-edge-subtle">
                <span className="text-[11px] font-bold text-content-secondary">Memory</span>
                <span className="text-[11px] font-mono text-content-tertiary">{memoryPlugin?.name || agent.metadata?.preferred_memory || 'None'}</span>
              </div>
              <div className="flex items-center justify-between py-2 border-b border-edge-subtle">
                <span className="text-[11px] font-bold text-content-secondary">Type</span>
                <span className="inline-flex items-center gap-1.5 text-[11px] font-mono px-2 py-0.5 rounded-full" style={{ backgroundColor: `${color}12`, color }}>
                  <Cpu size={10} />
                  Container
                </span>
              </div>
              <div className="flex items-center justify-between py-2">
                <span className="text-[11px] font-bold text-content-secondary">Power</span>
                <span className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded text-[10px] font-bold ${statusBadgeClass(agent.status)}`}>
                  {agent.metadata?.has_power_password === 'true' && <Lock size={8} />}
                  {agent.status.toUpperCase()}
                </span>
              </div>
            </div>
          </div>

          {/* Actions */}
          <div className="flex gap-3">
            <button
              onClick={onConfigure}
              className="flex-1 flex items-center justify-center gap-2 py-3 rounded-xl text-white text-xs font-bold shadow-lg transition-all hover:shadow-xl active:scale-[0.98]"
              style={{ backgroundColor: color, boxShadow: `0 10px 15px -3px ${color}33` }}
            >
              <Puzzle size={14} />
              Manage Plugins
            </button>
          </div>

          {/* Info Notice */}
          <div className="flex items-start gap-3 p-4 bg-surface-base rounded-xl border border-edge-subtle">
            <Terminal size={14} className="text-content-tertiary shrink-0 mt-0.5" />
            <p className="text-[10px] text-content-tertiary leading-relaxed">
              This is a non-AI container agent. It operates through bridge scripts and does not support interactive chat.
              Use the plugin workspace to configure its engine and memory modules.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
