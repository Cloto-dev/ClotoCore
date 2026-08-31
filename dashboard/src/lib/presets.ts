/** Server preset definitions shared between SetupWizard and AgentConfig. */

import { Box, Layers, type LucideIcon, Shield, Zap } from 'lucide-react';
import type { MarketplaceCatalogEntry, MarketplaceCollection, McpServerInfo } from '../types';
import { isEngineServer } from './serverCategory';

// Server ids are bare (the category prefixes are retired;
// docs/CATEGORY_PREFIX_RETIREMENT_DESIGN.md).
//
// Every id below MUST exist in the live ClotoHub catalog
// (https://hub.cloto.dev/api/catalog) — batch-install silently skips unknown
// ids (bug-381), so a stale id here degrades first-run setup. Reconciled
// against the catalog on 2026-07-17 (removed: agent_utils, cron, imagegen,
// embedding → cembedding, stt, capture, gaze — none are published on the
// hub). The structural fix (hub-served collections) is tracked in
// docs/ONBOARDING_MODERNIZATION_DESIGN.md §3.
export const MINIMAL_SERVERS = ['cpersona'];

export const STANDARD_SERVERS = ['cpersona', 'terminal', 'websearch'];

export const ADVANCED_SERVERS = [...STANDARD_SERVERS, 'cembedding'];

export const EXPERT_SERVERS = [...ADVANCED_SERVERS, 'cscheduler'];

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

/** Lucide components for the hub's icon name hints. */
const COLLECTION_ICONS: Record<string, LucideIcon> = {
  box: Box,
  layers: Layers,
  shield: Shield,
  zap: Zap,
};

const FALLBACK_ENGINE = 'cerebras';

/**
 * Resolve hub-served collections into preset definitions, intersecting each
 * with the catalog ids actually present (ghost-proof — defense in depth on
 * top of the hub's own serve-time filter). Returns null when the hub gave
 * nothing usable, so callers keep the bundled SERVER_PRESETS floor
 * (docs/ONBOARDING_MODERNIZATION_DESIGN.md §3).
 */
export function resolveHubPresets(
  collections: MarketplaceCollection[] | undefined,
  catalog: MarketplaceCatalogEntry[],
): PresetInfo[] | null {
  if (!collections?.length) return null;
  const present = new Set(catalog.map((e) => e.id));
  const resolved: PresetInfo[] = [];
  for (const c of collections) {
    const servers = c.servers.filter((s) => present.has(s));
    if (servers.length === 0) continue;
    resolved.push({
      id: c.id,
      icon: COLLECTION_ICONS[c.icon] ?? Box,
      defaultEngine: c.default_engine && present.has(c.default_engine) ? c.default_engine : FALLBACK_ENGINE,
      servers,
    });
  }
  return resolved.length > 0 ? resolved : null;
}

/**
 * Detect which preset matches the current granted set (ignoring engines).
 * Returns the preset id or null if no exact match.
 *
 * Engines are excluded via the tool-surface `isEngineServer` test. A granted
 * id with no matching installed server cannot be classified (bare ids carry
 * no category information) and is treated as a preset server — if it isn't
 * one, the comparison simply finds no preset, which is the honest answer.
 */
export function detectPreset(
  grantedIds: Set<string>,
  servers: McpServerInfo[] = [],
  presets: PresetInfo[] = SERVER_PRESETS,
): string | null {
  const isEngine = (id: string): boolean => {
    const server = servers.find((s) => s.id === id);
    return server ? isEngineServer(server) : false;
  };
  const nonEngine = [...grantedIds].filter((id) => !isEngine(id));
  const sorted = [...nonEngine].sort().join(',');
  for (const preset of presets) {
    // bug-482: sort a copy — preset.servers aliases the exported *_SERVERS
    // constants, so an in-place .sort() permanently reorders shared state.
    if ([...preset.servers].sort().join(',') === sorted) return preset.id;
  }
  return null;
}
