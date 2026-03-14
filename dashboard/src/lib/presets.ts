/** Server preset definitions shared between SetupWizard and AgentConfig. */

import { Layers, Zap, Shield, Box, type LucideIcon } from 'lucide-react';

export const MINIMAL_SERVERS = ['memory.cpersona', 'tool.agent_utils'];

export const STANDARD_SERVERS = [
  'memory.cpersona', 'tool.cron', 'tool.terminal',
  'tool.websearch', 'tool.research', 'tool.agent_utils',
];

export const ADVANCED_SERVERS = [
  ...STANDARD_SERVERS,
  'tool.imagegen', 'vision.capture', 'tool.embedding',
];

export const EXPERT_SERVERS = [
  ...ADVANCED_SERVERS,
  'vision.gaze_webcam', 'voice.stt', 'voice.tts',
];

export interface PresetInfo {
  id: string;
  servers: string[];
  icon: LucideIcon;
  defaultEngine: string;
}

export const SERVER_PRESETS: PresetInfo[] = [
  { id: 'standard', icon: Layers, defaultEngine: 'mind.cerebras', servers: STANDARD_SERVERS },
  { id: 'advanced', icon: Zap,    defaultEngine: 'mind.deepseek', servers: ADVANCED_SERVERS },
  { id: 'expert',   icon: Shield, defaultEngine: 'mind.deepseek', servers: EXPERT_SERVERS },
  { id: 'minimal',  icon: Box,    defaultEngine: 'mind.cerebras', servers: MINIMAL_SERVERS },
];

/**
 * Detect which preset matches the current granted set (ignoring mind.* engines).
 * Returns the preset id or null if no exact match.
 */
export function detectPreset(grantedIds: Set<string>): string | null {
  const nonEngine = [...grantedIds].filter(id => !id.startsWith('mind.'));
  const sorted = nonEngine.sort().join(',');
  for (const preset of SERVER_PRESETS) {
    if (preset.servers.sort().join(',') === sorted) return preset.id;
  }
  return null;
}
