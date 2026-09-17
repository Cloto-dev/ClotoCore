import { ArrowLeft, Cpu, Database, Server } from 'lucide-react';
import type React from 'react';
import { useAgents } from '../hooks/useAgents';
import { StatusDot } from './ui/StatusDot';

interface KernelMonitorProps {
  onClose: () => void;
}

export const KernelMonitor: React.FC<KernelMonitorProps> = ({ onClose }) => {
  const { agents } = useAgents();

  const enabledAgents = agents.filter((a) => a.enabled);

  return (
    <div className="flex flex-col h-full bg-surface-panel p-6 overflow-hidden animate-in fade-in duration-300">
      <header className="mb-8 flex items-center justify-between">
        <div className="flex items-center gap-4">
          <button
            onClick={onClose}
            className="p-2.5 rounded-full bg-surface-control border border-edge hover:border-agent hover:text-agent transition-all"
          >
            <ArrowLeft size={18} />
          </button>
          <div className="w-10 h-10 bg-surface-control rounded-md flex items-center justify-center shadow-sm border border-edge">
            <Cpu className="text-agent" size={20} />
          </div>
          <div>
            <h1 className="text-2xl font-black text-content-primary">Kernel Monitor</h1>
            <p className="text-xs text-content-tertiary font-mono">System Core Status</p>
          </div>
        </div>
        <div className="bg-surface-control px-4 py-2 rounded-md shadow-sm border border-edge">
          <span className="text-sm font-mono font-bold text-agent">v{__APP_VERSION__}</span>
        </div>
      </header>

      <div className="grid grid-cols-3 gap-4 px-4 mb-8">
        {[
          { label: 'Agents', val: agents.length, icon: Cpu },
          { label: 'Online', val: enabledAgents.length, icon: Database },
          { label: 'Offline', val: agents.length - enabledAgents.length, icon: Server },
        ].map((stat) => (
          <div key={stat.label} className="bg-surface-field p-4 rounded-2xl border border-edge-subtle shadow-sm">
            <div className="flex items-center gap-2 mb-2">
              <stat.icon size={14} className="text-agent" />
              <span className="text-xs font-black text-content-tertiary">{stat.label}</span>
            </div>
            <div className="text-xl font-mono font-bold text-content-primary">{stat.val}</div>
          </div>
        ))}
      </div>

      <div className="flex-1 overflow-y-auto no-scrollbar px-4 space-y-6">
        <section>
          <div className="flex items-center gap-2 mb-3">
            <div className="w-1 h-3 bg-emerald-500 rounded-full" />
            <h3 className="text-xs font-black text-content-secondary">Live Neural Nodes</h3>
          </div>
          <div className="flex flex-wrap gap-2">
            {agents.map((agent) => (
              <div
                key={agent.id}
                className={`px-4 py-2 border rounded-xl flex items-center gap-3 ${agent.enabled ? 'bg-emerald-500/10 border-emerald-500/20' : 'bg-surface-secondary border-edge-subtle opacity-60'}`}
              >
                <StatusDot status={agent.enabled ? 'online' : 'offline'} pulse={agent.enabled} />
                <span className={`text-xs font-bold ${agent.enabled ? 'text-emerald-700' : 'text-content-tertiary'}`}>
                  {agent.name}
                </span>
              </div>
            ))}
            {agents.length === 0 && (
              <div className="text-xs text-content-tertiary font-mono italic">No agents detected.</div>
            )}
          </div>
        </section>
      </div>

      <div className="px-4 py-4 mt-4"></div>
    </div>
  );
};
