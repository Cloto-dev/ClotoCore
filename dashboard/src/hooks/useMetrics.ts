import { useState, useEffect, useCallback } from 'react';
import { api } from '../services/api';

export interface Metrics {
  total_requests: number;
  total_memories: number;
  total_episodes: number;
  ram_usage: string;
}

export function useMetrics(pollIntervalMs: number = 10000) {
  const [metrics, setMetrics] = useState<Metrics | null>(null);

  const fetchMetrics = useCallback(async () => {
    try {
      setMetrics(await api.getMetrics());
    } catch (e) {
      console.error("Failed to fetch metrics", e);
    }
  }, []);

  useEffect(() => {
    fetchMetrics();
    const interval = setInterval(fetchMetrics, pollIntervalMs);
    return () => clearInterval(interval);
  }, [fetchMetrics, pollIntervalMs]);

  return { metrics, fetchMetrics };
}
