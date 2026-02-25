import { Brain, Cpu, Database, Globe, MousePointer2, Puzzle, Zap } from 'lucide-react';
import { PluginManifest } from '../types';

/** Check if a plugin is an LLM-powered engine */
export function isLlmPlugin(p: PluginManifest): boolean {
  return p.tags.includes('#MIND') || p.tags.includes('#LLM');
}

/** Render icon for a plugin's service_type */
export function ServiceTypeIcon({ type, size = 20 }: { type: string; size?: number }) {
  switch (type) {
    case 'Reasoning': return <Brain size={size} />;
    case 'Memory': return <Database size={size} />;
    case 'Skill': return <Zap size={size} />;
    case 'Action': case 'HAL': return <MousePointer2 size={size} />;
    case 'Communication': return <Globe size={size} />;
    default: return <Puzzle size={size} />;
  }
}
