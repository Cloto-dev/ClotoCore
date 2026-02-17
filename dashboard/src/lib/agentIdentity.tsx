import React from 'react';
import { User, Cpu } from 'lucide-react';
import { AgentMetadata } from '../types';

export type AgentType = 'ai' | 'container';

/** AI agent accent color (blue) */
export const AI_AGENT_COLOR = '#2e4de6';
/** Non-AI container accent color (rose-pink) */
export const CONTAINER_COLOR = '#e6466a';

/**
 * Determine whether an agent is an AI agent or a non-AI container.
 * Priority: metadata.agent_type (explicit) > engine prefix (auto-detect)
 */
export function isAiAgent(agent: AgentMetadata): boolean {
  if (agent.metadata?.agent_type === 'ai') return true;
  if (agent.metadata?.agent_type === 'container') return false;
  return !!agent.default_engine_id?.startsWith('mind.');
}

/** Get the resolved agent type */
export function getAgentType(agent: AgentMetadata): AgentType {
  return isAiAgent(agent) ? 'ai' : 'container';
}

/** Get the accent color for an agent */
export function agentColor(agent: AgentMetadata): string {
  return isAiAgent(agent) ? AI_AGENT_COLOR : CONTAINER_COLOR;
}

/** Get accent color by type directly (useful in creation forms) */
export function agentTypeColor(type: AgentType): string {
  return type === 'ai' ? AI_AGENT_COLOR : CONTAINER_COLOR;
}

/** Render the appropriate icon for an agent */
export function AgentIcon({ agent, size = 20 }: { agent: AgentMetadata; size?: number }) {
  return isAiAgent(agent) ? <User size={size} /> : <Cpu size={size} />;
}

/** Render icon by type directly (useful in creation forms) */
export function AgentTypeIcon({ type, size = 20 }: { type: AgentType; size?: number }) {
  return type === 'ai' ? <User size={size} /> : <Cpu size={size} />;
}

/** Status dot color classes (3-state) */
export function statusDotColor(status: string): string {
  return status === 'online' ? 'bg-emerald-500' :
         status === 'degraded' ? 'bg-amber-500 animate-pulse' : 'bg-slate-300';
}

/** Status badge classes (3-state) */
export function statusBadgeClass(status: string): string {
  return status === 'online' ? 'bg-emerald-50 text-emerald-500' :
         status === 'degraded' ? 'bg-amber-50 text-amber-500' : 'bg-slate-100 text-slate-400';
}
