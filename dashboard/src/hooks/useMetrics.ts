import { useState, useCallback } from 'react';
import { useApi } from './useApi';
import { usePolling } from './usePolling';

export interface Metrics {
  total_requests: number;
  total_memories: number;
  total_episodes: number;
  ram_usage: string;
}

export function useMetrics(pollIntervalMs: number = 10000) {
  const api = useApi();
  const [metrics, setMetrics] = useState<Metrics | null>(null);

  const fetchMetrics = useCallback(async () => {
    try {
      setMetrics(await api.getMetrics());
    } catch (e) {
      console.error("Failed to fetch metrics", e);
    }
  }, [api]);

  usePolling(fetchMetrics, pollIntervalMs);

  return { metrics, fetchMetrics };
}
