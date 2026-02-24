import { Power } from 'lucide-react';
import { AgentMetadata } from '../types';

/** Unified power button: always click → confirmation dialog */
export function AgentPowerButton({ agent, onPowerToggle }: {
  agent: AgentMetadata;
  onPowerToggle: (agent: AgentMetadata) => void;
}) {
  const isOn = agent.enabled;

  return (
    <button
      className={`inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg border text-[10px] font-bold uppercase tracking-wider transition-all ${
        isOn
          ? 'border-emerald-500/30 text-emerald-500 bg-emerald-500/10 hover:bg-emerald-500/20'
          : 'border-edge text-content-muted hover:bg-surface-secondary'
      }`}
      title={isOn ? 'Power Off' : 'Power On'}
      onClick={(e) => { e.stopPropagation(); onPowerToggle(agent); }}
    >
      <Power size={12} />
      {isOn ? 'ON' : 'OFF'}
    </button>
  );
}
