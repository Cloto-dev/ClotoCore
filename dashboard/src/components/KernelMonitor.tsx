import React from 'react';
import { Cpu, Database, MousePointer2, Globe } from 'lucide-react';
import { ServiceTypeIcon } from '../lib/pluginUtils';
import { usePlugins } from '../hooks/usePlugins';
import { useAgents } from '../hooks/useAgents';

interface KernelMonitorProps {
  onClose: () => void;
}

export const KernelMonitor: React.FC<KernelMonitorProps> = ({ onClose }) => {
  const { plugins, isLoading: pluginsLoading } = usePlugins();
  const { agents, isLoading: agentsLoading } = useAgents();
  const isLoading = pluginsLoading || agentsLoading;

  const activePlugins = plugins.filter(p => p.is_active);
  const capabilityStats = {
    reasoning: activePlugins.filter(p => p.provided_capabilities.includes('Reasoning')).length,
    memory: activePlugins.filter(p => p.provided_capabilities.includes('Memory')).length,
    hal: activePlugins.filter(p => p.provided_capabilities.includes('HAL')).length,
    comms: activePlugins.filter(p => p.provided_capabilities.includes('Communication')).length,
  };


  return (
    <div className="flex flex-col h-full bg-glass backdrop-blur-3xl p-6 overflow-hidden animate-in fade-in duration-300">
      <div className="mb-8 px-4 flex justify-between items-start">
        <div>
          <h2 className="text-2xl font-black tracking-tighter text-content-primary uppercase leading-none">Kernel Monitor</h2>
          <p className="text-[10px] text-content-tertiary font-mono tracking-[0.3em] uppercase mt-2">Exiv System Core Status</p>
        </div>
        <div className="px-3 py-1 bg-brand/10 text-brand rounded-full text-[9px] font-bold font-mono">
          v{__APP_VERSION__}
        </div>
      </div>

      <div className="grid grid-cols-4 gap-4 px-4 mb-8">
        {[
          { label: 'Reasoning', val: capabilityStats.reasoning, icon: Cpu },
          { label: 'Memory', val: capabilityStats.memory, icon: Database },
          { label: 'HAL', val: capabilityStats.hal, icon: MousePointer2 },
          { label: 'Communication', val: capabilityStats.comms, icon: Globe },
        ].map(stat => (
          <div key={stat.label} className="bg-glass-strong p-4 rounded-2xl border border-edge-subtle shadow-sm">
            <div className="flex items-center gap-2 mb-2">
              <stat.icon size={14} className="text-brand" />
              <span className="text-[8px] font-black text-content-tertiary uppercase tracking-widest">{stat.label}</span>
            </div>
            <div className="text-xl font-mono font-bold text-content-primary">{stat.val}</div>
          </div>
        ))}
      </div>

      {/* Capability Map */}
      <div className="flex-1 overflow-y-auto no-scrollbar px-4 space-y-6">
        <section>
          <div className="flex items-center gap-2 mb-3">
             <div className="w-1 h-3 bg-brand rounded-full" />
             <h3 className="text-[10px] font-black text-content-secondary uppercase tracking-widest">Active Capability Providers</h3>
          </div>
          <div className="space-y-2">
            {activePlugins.map(plugin => (
              <div key={plugin.id} className="p-3 bg-glass border border-glass-strong rounded-xl flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div className="p-1.5 bg-surface-primary rounded-lg text-content-tertiary">
                    <ServiceTypeIcon type={plugin.service_type} size={12} />
                  </div>
                  <div>
                    <div className="text-xs font-bold text-content-primary">{plugin.name}</div>
                    <div className="flex gap-1 mt-1">
                      {plugin.provided_capabilities.map(cap => (
                        <span key={cap} className="text-[8px] bg-brand/5 text-brand px-1.5 py-0.5 rounded uppercase font-bold">{cap}</span>
                      ))}
                    </div>
                  </div>
                </div>
                <div className="text-[9px] font-mono text-content-tertiary">
                  {plugin.required_permissions.length} PERMS
                </div>
              </div>
            ))}
          </div>
        </section>

        <section>
          <div className="flex items-center gap-2 mb-3">
             <div className="w-1 h-3 bg-emerald-500 rounded-full" />
             <h3 className="text-[10px] font-black text-content-secondary uppercase tracking-widest">Live Neural Nodes</h3>
          </div>
          <div className="flex flex-wrap gap-2">
            {agents.map(agent => (
              <div key={agent.id} className="px-4 py-2 bg-emerald-500/10 border border-emerald-500/20 rounded-xl flex items-center gap-3">
                <div className="w-2 h-2 rounded-full bg-emerald-500 animate-pulse" />
                <span className="text-xs font-bold text-emerald-700 uppercase tracking-wider">{agent.name}</span>
              </div>
            ))}
            {agents.length === 0 && <div className="text-[10px] text-content-tertiary font-mono italic">No active agents detected.</div>}
          </div>
        </section>
      </div>

      <div className="px-4 py-4 border-t border-edge-subtle mt-4">
        <button
          onClick={onClose}
          className="w-full py-3 bg-surface-secondary text-brand rounded-xl text-[10px] font-black uppercase tracking-[0.2em] hover:bg-brand hover:text-white transition-all shadow-sm active:scale-95"
        >
          CLOSE KERNEL MONITOR
        </button>
      </div>
    </div>
  );
};
