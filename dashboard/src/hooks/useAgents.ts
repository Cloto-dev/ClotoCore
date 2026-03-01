import { useState, useEffect, useCallback } from 'react';
import { AgentMetadata } from '../types';
import { api } from '../services/api';

/** Module-level cache for deduplication across components */
let cache: { data: AgentMetadata[]; ts: number } | null = null;
let inflight: Promise<AgentMetadata[]> | null = null;
const TTL = 10_000; // 10 seconds

async function fetchCached(): Promise<AgentMetadata[]> {
  if (cache && Date.now() - cache.ts < TTL) return cache.data;
  if (inflight) return inflight;

  inflight = api.getAgents().then(data => {
    cache = { data, ts: Date.now() };
    inflight = null;
    return data;
  }).catch(err => {
    inflight = null;
    throw err;
  });
  return inflight;
}

export function useAgents() {
  const [data, setData] = useState<AgentMetadata[]>(cache?.data ?? []);
  const [isLoading, setIsLoading] = useState(!cache);
  const [error, setError] = useState<string | null>(null);

  const refetch = useCallback(async () => {
    cache = null; // invalidate
    setIsLoading(true);
    setError(null);
    try {
      const result = await fetchCached();
      setData(result);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to fetch agents';
      setError(msg);
      console.error('Failed to fetch agents:', err);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchCached()
      .then(setData)
      .catch(err => {
        const msg = err instanceof Error ? err.message : 'Failed to fetch agents';
        setError(msg);
        console.error('Failed to fetch agents:', err);
      })
      .finally(() => setIsLoading(false));
  }, []);

  return { agents: data, isLoading, error, refetch };
}
