import { Power } from 'lucide-react';
import { AgentMetadata } from '../types';
import { useLongPress } from '../hooks/useLongPress';

function LongPressPowerButton({ agent, onComplete }: { agent: AgentMetadata; onComplete: (agent: AgentMetadata) => void }) {
  const durationMs = agent.enabled ? 2000 : 1000;
  const { progress, handlers } = useLongPress(durationMs, () => onComplete(agent));

  const isOn = agent.enabled;
  const progressColor = isOn ? 'bg-red-400/25' : 'bg-emerald-400/25';
  const ringColor = isOn
    ? (progress > 0 ? 'border-red-500/30 text-red-500' : 'border-emerald-500/30 text-emerald-500')
    : (progress > 0 ? 'border-emerald-500/30 text-emerald-500' : 'border-edge text-content-tertiary');

  return (
    <button
      {...handlers}
      onMouseDown={(e) => { e.stopPropagation(); handlers.onMouseDown(); }}
      onTouchStart={(e) => { e.stopPropagation(); handlers.onTouchStart(); }}
      onClick={(e) => e.stopPropagation()}
      className={`relative p-2 rounded-lg border transition-all overflow-hidden ${ringColor} ${
        isOn ? 'hover:bg-emerald-500/10' : 'hover:bg-surface-base'
      }`}
      title={isOn ? `Power Off (hold ${durationMs / 1000}s)` : `Power On (hold ${durationMs / 1000}s)`}
    >
      {progress > 0 && (
        <span
          className={`absolute inset-0 ${progressColor} origin-left transition-none`}
          style={{ transform: `scaleX(${progress})` }}
        />
      )}
      <Power size={16} className="relative" />
    </button>
  );
}

/** Unified power button: shows password-modal button or long-press button */
export function AgentPowerButton({ agent, onPowerToggle }: {
  agent: AgentMetadata;
  onPowerToggle: (agent: AgentMetadata) => void;
}) {
  if (agent.metadata?.has_power_password === 'true') {
    return (
      <button
        className={`p-2 rounded-lg border transition-all ${
          agent.enabled
            ? 'border-emerald-500/30 text-emerald-500 hover:bg-emerald-500/10'
            : 'border-edge text-content-tertiary hover:bg-surface-base'
        }`}
        title={agent.enabled ? 'Power Off' : 'Power On'}
        onClick={(e) => { e.stopPropagation(); onPowerToggle(agent); }}
      >
        <Power size={16} />
      </button>
    );
  }

  return <LongPressPowerButton agent={agent} onComplete={onPowerToggle} />;
}
