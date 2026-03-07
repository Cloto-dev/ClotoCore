import { useState } from 'react';
import { User } from 'lucide-react';
import { AgentMetadata } from '../types';
import { api } from '../services/api';

/** Get current brand hex from CSS variable */
function getBrandHex(): string {
  return getComputedStyle(document.documentElement).getPropertyValue('--brand-hex').trim() || '#2e4de6';
}

/** Get the accent color for an agent */
export function agentColor(_agent: AgentMetadata): string {
  return getBrandHex();
}

/** Render the appropriate icon for an agent (avatar image or fallback icon) */
export function AgentIcon({ agent, size = 20 }: { agent: AgentMetadata; size?: number }) {
  const [imgError, setImgError] = useState(false);

  if (agent.metadata?.has_avatar === 'true' && !imgError) {
    return (
      <img
        src={api.getAvatarUrl(agent.id)}
        alt={agent.name}
        className="rounded-md object-cover"
        style={{ width: size, height: size }}
        onError={() => setImgError(true)}
      />
    );
  }
  return <User size={size} />;
}

/** Status dot color classes (3-state) */
export function statusDotColor(status: string): string {
  return status === 'online' ? 'bg-emerald-500' :
         status === 'degraded' ? 'bg-amber-500 animate-pulse' : 'bg-content-muted';
}

/** Status badge classes (3-state) */
export function statusBadgeClass(status: string): string {
  return status === 'online' ? 'bg-emerald-500/10 text-emerald-500' :
         status === 'degraded' ? 'bg-amber-500/10 text-amber-500' : 'bg-surface-secondary text-content-tertiary';
}
