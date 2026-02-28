import React from 'react';
import { Cpu, Database, MousePointer2, Globe, ArrowLeft, Server } from 'lucide-react';
import { useAgents } from '../hooks/useAgents';

interface KernelMonitorProps {
  onClose: () => void;
}

export const KernelMonitor: React.FC<KernelMonitorProps> = ({ onClose }) => {
  const { agents, isLoading } = useAgents();

  const enabledAgents = agents.filter(a => a.enabled);

  return (
    <div className="flex flex-col h-full bg-glass backdrop-blur-3xl p-6 overflow-hidden animate-in fade-in duration-300">
      <header className="mb-8 flex items-center justify-between">
        <div className="flex items-center gap-4">
          <button
            onClick={onClose}
            className="p-2.5 rounded-full bg-glass-subtle backdrop-blur-sm border border-edge hover:border-brand hover:text-brand transition-all"
          >
            <ArrowLeft size={18} />
          </button>
          <div className="w-10 h-10 bg-glass-subtle backdrop-blur-sm rounded-md flex items-center justify-center shadow-sm border border-edge">
            <Cpu className="text-brand" size={20} />
          </div>
          <div>
            <h1 className="text-2xl font-black tracking-tighter text-content-primary uppercase">Kernel Monitor</h1>
            <p className="text-[10px] text-content-tertiary font-mono uppercase tracking-[0.2em]">System Core Status</p>
          </div>
        </div>
        <div className="bg-glass-subtle backdrop-blur-sm px-4 py-2 rounded-md shadow-sm border border-edge">
          <span className="text-sm font-mono font-bold text-brand">v{__APP_VERSION__}</span>
        </div>
      </header>

      <div className="grid grid-cols-3 gap-4 px-4 mb-8">
        {[
          { label: 'Agents', val: agents.length, icon: Cpu },
          { label: 'Online', val: enabledAgents.length, icon: Database },
          { label: 'Offline', val: agents.length - enabledAgents.length, icon: Server },
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

      <div className="flex-1 overflow-y-auto no-scrollbar px-4 space-y-6">
        <section>
          <div className="flex items-center gap-2 mb-3">
             <div className="w-1 h-3 bg-emerald-500 rounded-full" />
             <h3 className="text-[10px] font-black text-content-secondary uppercase tracking-widest">Live Neural Nodes</h3>
          </div>
          <div className="flex flex-wrap gap-2">
            {agents.map(agent => (
              <div key={agent.id} className={`px-4 py-2 border rounded-xl flex items-center gap-3 ${agent.enabled ? 'bg-emerald-500/10 border-emerald-500/20' : 'bg-surface-secondary border-edge-subtle opacity-60'}`}>
                <div className={`w-2 h-2 rounded-full ${agent.enabled ? 'bg-emerald-500 animate-pulse' : 'bg-gray-500'}`} />
                <span className={`text-xs font-bold uppercase tracking-wider ${agent.enabled ? 'text-emerald-700' : 'text-content-tertiary'}`}>{agent.name}</span>
              </div>
            ))}
            {agents.length === 0 && <div className="text-[10px] text-content-tertiary font-mono italic">No agents detected.</div>}
          </div>
        </section>
      </div>

      <div className="px-4 py-4 mt-4">
      </div>
    </div>
  );
};
