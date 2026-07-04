/** Server preset definitions shared between SetupWizard and AgentConfig. */

import { Box, Layers, type LucideIcon, Shield, Zap } from 'lucide-react';
import type { McpServerInfo } from '../types';
import { isEngineServer } from './serverCategory';

// Server ids are bare (an earlier decision — the category prefixes are retired;
// docs/CATEGORY_PREFIX_RETIREMENT_DESIGN.md).
export const MINIMAL_SERVERS = ['cpersona', 'agent_utils'];

export const STANDARD_SERVERS = ['cpersona', 'cron', 'terminal', 'websearch', 'agent_utils'];

export const ADVANCED_SERVERS = [...STANDARD_SERVERS, 'imagegen', 'embedding'];

export const EXPERT_SERVERS = [...ADVANCED_SERVERS, 'stt', 'capture', 'gaze'];

export interface PresetInfo {
  id: string;
  servers: string[];
  icon: LucideIcon;
  defaultEngine: string;
}

export const SERVER_PRESETS: PresetInfo[] = [
  { id: 'standard', icon: Layers, defaultEngine: 'cerebras', servers: STANDARD_SERVERS },
  { id: 'advanced', icon: Zap, defaultEngine: 'deepseek', servers: ADVANCED_SERVERS },
  { id: 'expert', icon: Shield, defaultEngine: 'deepseek', servers: EXPERT_SERVERS },
  { id: 'minimal', icon: Box, defaultEngine: 'cerebras', servers: MINIMAL_SERVERS },
];

/**
 * Detect which preset matches the current granted set (ignoring engines).
 * Returns the preset id or null if no exact match.
 *
 * Engines are excluded via the tool-surface `isEngineServer` test. A granted
 * id with no matching installed server cannot be classified (bare ids carry
 * no category information) and is treated as a preset server — if it isn't
 * one, the comparison simply finds no preset, which is the honest answer.
 */
export function detectPreset(grantedIds: Set<string>, servers: McpServerInfo[] = []): string | null {
  const isEngine = (id: string): boolean => {
    const server = servers.find((s) => s.id === id);
    return server ? isEngineServer(server) : false;
  };
  const nonEngine = [...grantedIds].filter((id) => !isEngine(id));
  const sorted = nonEngine.sort().join(',');
  for (const preset of SERVER_PRESETS) {
    if (preset.servers.sort().join(',') === sorted) return preset.id;
  }
  return null;
}
