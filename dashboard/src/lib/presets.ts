/** Server preset definitions shared between SetupWizard and AgentConfig. */

import { Box, Layers, type LucideIcon, Shield, Zap } from 'lucide-react';
import type { McpServerInfo } from '../types';
import { isEngineServer } from './serverCategory';

export const MINIMAL_SERVERS = ['cpersona', 'tool.agent_utils'];

export const STANDARD_SERVERS = ['cpersona', 'tool.cron', 'tool.terminal', 'tool.websearch', 'tool.agent_utils'];

export const ADVANCED_SERVERS = [...STANDARD_SERVERS, 'tool.imagegen', 'tool.embedding'];

export const EXPERT_SERVERS = [...ADVANCED_SERVERS, 'voice.stt', 'vision.capture', 'vision.gaze_webcam'];

export interface PresetInfo {
  id: string;
  servers: string[];
  icon: LucideIcon;
  defaultEngine: string;
}

export const SERVER_PRESETS: PresetInfo[] = [
  { id: 'standard', icon: Layers, defaultEngine: 'mind.cerebras', servers: STANDARD_SERVERS },
  { id: 'advanced', icon: Zap, defaultEngine: 'mind.deepseek', servers: ADVANCED_SERVERS },
  { id: 'expert', icon: Shield, defaultEngine: 'mind.deepseek', servers: EXPERT_SERVERS },
  { id: 'minimal', icon: Box, defaultEngine: 'mind.cerebras', servers: MINIMAL_SERVERS },
];

/**
 * Detect which preset matches the current granted set (ignoring engines).
 * Returns the preset id or null if no exact match.
 *
 * Engines are excluded via `isEngineServer` so de-prefixed ClotoHub catalog
 * engines (e.g. `deepseek`, which lacks the legacy `mind.` prefix — bug-388/396)
 * are not counted as preset servers. Falls back to prefix matching when a
 * granted id has no known server object.
 */
export function detectPreset(grantedIds: Set<string>, servers: McpServerInfo[] = []): string | null {
  const isEngine = (id: string): boolean => {
    const server = servers.find((s) => s.id === id);
    return server ? isEngineServer(server) : id.startsWith('mind.');
  };
  const nonEngine = [...grantedIds].filter((id) => !isEngine(id));
  const sorted = nonEngine.sort().join(',');
  for (const preset of SERVER_PRESETS) {
    if (preset.servers.sort().join(',') === sorted) return preset.id;
  }
  return null;
}
