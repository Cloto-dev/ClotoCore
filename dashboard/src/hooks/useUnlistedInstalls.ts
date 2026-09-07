import { useEffect, useState } from 'react';
import type { UnlistedInstall } from '../types';
import { useApi } from './useApi';

/**
 * Installs no entry in the current catalog accounts for.
 *
 * These ride on the catalog response rather than an endpoint of their own: the
 * kernel derives them from the same rows it already read to decide what is
 * installed, so both directions run the same lookups and cannot contradict
 * each other. The hook mirrors `useHubPresets`, which reads the same response
 * for a different part of it.
 *
 * An empty result is not proof of a clean install set — the kernel reports
 * nothing when it has no catalog to compare against, because with no catalog
 * every install would look retired.
 */
export function useUnlistedInstalls(): UnlistedInstall[] {
  const api = useApi();
  const [unlisted, setUnlisted] = useState<UnlistedInstall[]>([]);

  useEffect(() => {
    if (!api.apiKey) return;
    let alive = true;
    api
      .getMarketplaceCatalog()
      .then((cat) => {
        if (!alive) return;
        setUnlisted(cat.unlisted_installs ?? []);
      })
      .catch(() => {
        // Catalog unavailable — say nothing rather than guess.
      });
    return () => {
      alive = false;
    };
  }, [api]);

  return unlisted;
}
