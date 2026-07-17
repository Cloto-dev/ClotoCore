import { useEffect, useState } from 'react';
import { type PresetInfo, resolveHubPresets, SERVER_PRESETS } from '../lib/presets';
import { useApi } from './useApi';

/**
 * Preset resolution chain (docs/ONBOARDING_MODERNIZATION_DESIGN.md §3):
 * live hub catalog (kernel-cached, Stale-tolerant) → bundled SERVER_PRESETS
 * as the offline floor. The hook renders the floor immediately and swaps in
 * hub-served collections when (and only when) they resolve to something
 * usable, so setup never blocks on the network.
 */
export function useHubPresets(): PresetInfo[] {
  const api = useApi();
  const [presets, setPresets] = useState<PresetInfo[]>(SERVER_PRESETS);

  useEffect(() => {
    // No key yet (e.g. wizard before the key handover resolves, or browser
    // mode before login) — keep the bundled floor.
    if (!api.apiKey) return;
    let alive = true;
    api
      .getMarketplaceCatalog()
      .then((cat) => {
        if (!alive) return;
        const resolved = resolveHubPresets(cat.collections, cat.servers);
        if (resolved) setPresets(resolved);
      })
      .catch(() => {
        // Offline / catalog unavailable — the bundled floor stays.
      });
    return () => {
      alive = false;
    };
  }, [api]);

  return presets;
}
