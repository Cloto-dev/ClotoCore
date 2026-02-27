import { useState, useEffect, useCallback } from 'react';
import { McpServerInfo } from '../types';
import { api } from '../services/api';

/** Module-level cache for deduplication across components */
let cache: { data: McpServerInfo[]; ts: number } | null = null;
let inflight: Promise<McpServerInfo[]> | null = null;
const TTL = 10_000; // 10 seconds

async function fetchCached(apiKey: string): Promise<McpServerInfo[]> {
  if (cache && Date.now() - cache.ts < TTL) return cache.data;
  if (inflight) return inflight;

  inflight = api.listMcpServers(apiKey).then(data => {
    cache = { data: data.servers, ts: Date.now() };
    inflight = null;
    return data.servers;
  }).catch(err => {
    inflight = null;
    throw err;
  });
  return inflight;
}

export function useMcpServers(apiKey: string) {
  const [data, setData] = useState<McpServerInfo[]>(cache?.data ?? []);
  const [isLoading, setIsLoading] = useState(!cache);
  const [error, setError] = useState<string | null>(null);

  const refetch = useCallback(async () => {
    cache = null; // invalidate
    setIsLoading(true);
    setError(null);
    // Guarantee minimum spin duration so the user sees feedback
    const minSpin = new Promise(r => setTimeout(r, 400));
    try {
      const [result] = await Promise.all([fetchCached(apiKey), minSpin]);
      setData(result);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to connect';
      setError(msg);
      console.error('Failed to fetch MCP servers:', err);
    } finally {
      setIsLoading(false);
    }
  }, [apiKey]);

  useEffect(() => {
    fetchCached(apiKey)
      .then(d => { setData(d); setError(null); })
      .catch(err => {
        const msg = err instanceof Error ? err.message : 'Failed to connect';
        setError(msg);
        console.error('Failed to fetch MCP servers:', err);
      })
      .finally(() => setIsLoading(false));
  }, [apiKey]);

  return { servers: data, isLoading, error, refetch };
}
