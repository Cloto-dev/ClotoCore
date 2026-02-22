import { useState, useEffect, useCallback } from 'react';
import { PluginManifest } from '../types';
import { api } from '../services/api';

/** Module-level cache for deduplication across components */
let cache: { data: PluginManifest[]; ts: number } | null = null;
let inflight: Promise<PluginManifest[]> | null = null;
const TTL = 10_000; // 10 seconds

async function fetchCached(): Promise<PluginManifest[]> {
  if (cache && Date.now() - cache.ts < TTL) return cache.data;
  if (inflight) return inflight;

  inflight = api.getPlugins().then(data => {
    cache = { data, ts: Date.now() };
    inflight = null;
    return data;
  }).catch(err => {
    inflight = null;
    throw err;
  });
  return inflight;
}

export function usePlugins() {
  const [data, setData] = useState<PluginManifest[]>(cache?.data ?? []);
  const [isLoading, setIsLoading] = useState(!cache);

  const refetch = useCallback(async () => {
    cache = null; // invalidate
    setIsLoading(true);
    try {
      const result = await fetchCached();
      setData(result);
    } catch (err) {
      console.error('Failed to fetch plugins:', err);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchCached()
      .then(setData)
      .catch(err => console.error('Failed to fetch plugins:', err))
      .finally(() => setIsLoading(false));
  }, []);

  return { plugins: data, isLoading, refetch };
}
