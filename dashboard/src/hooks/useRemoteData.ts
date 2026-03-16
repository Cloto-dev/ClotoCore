import { useCallback, useEffect, useRef, useState } from 'react';
import { extractError } from '../lib/errors';

/* Module-level caches shared across all hook instances */
const caches = new Map<string, { data: unknown[]; ts: number }>();
const inflights = new Map<string, Promise<unknown[]>>();

export interface UseRemoteDataOptions {
  /** Unique cache key. Changing this triggers a re-fetch. */
  key: string;
  /** Cache TTL in milliseconds (default: 10_000). */
  ttl?: number;
  /** Fallback error message. */
  errorMessage?: string;
  /** Minimum visual feedback duration on manual refetch (default: 0). */
  minRefetchMs?: number;
  /** Alternative fetcher used on manual refetch (e.g. to bypass server cache). */
  refetcher?: () => Promise<unknown[]>;
}

export function useRemoteData<T>(
  fetcher: () => Promise<T[]>,
  options: UseRemoteDataOptions,
): { data: T[]; isLoading: boolean; error: string | null; refetch: () => Promise<void> } {
  const { key, ttl = 10_000, errorMessage = 'Failed to fetch', minRefetchMs = 0, refetcher } = options;

  // Keep fetcher up-to-date without causing effect re-runs
  const fetcherRef = useRef(fetcher);
  fetcherRef.current = fetcher;
  const refetcherRef = useRef(refetcher);
  refetcherRef.current = refetcher;

  const cached = caches.get(key) as { data: T[]; ts: number } | undefined;
  const [data, setData] = useState<T[]>(cached?.data ?? []);
  const [isLoading, setIsLoading] = useState(!cached);
  const [error, setError] = useState<string | null>(null);

  const fetchCached = useCallback((): Promise<T[]> => {
    const c = caches.get(key) as { data: T[]; ts: number } | undefined;
    if (c && Date.now() - c.ts < ttl) return Promise.resolve(c.data);

    const existing = inflights.get(key);
    if (existing) return existing as Promise<T[]>;

    const promise = fetcherRef
      .current()
      .then((result) => {
        caches.set(key, { data: result, ts: Date.now() });
        inflights.delete(key);
        return result;
      })
      .catch((err) => {
        inflights.delete(key);
        throw err;
      });

    inflights.set(key, promise as Promise<unknown[]>);
    return promise;
  }, [key, ttl]);

  const refetch = useCallback(async () => {
    caches.delete(key);
    inflights.delete(key);
    setIsLoading(true);
    setError(null);
    const minSpin = minRefetchMs > 0 ? new Promise<void>((r) => setTimeout(r, minRefetchMs)) : Promise.resolve();
    try {
      const doFetch = refetcherRef.current ? (refetcherRef.current() as Promise<T[]>) : fetchCached();
      const [result] = await Promise.all([doFetch, minSpin]);
      caches.set(key, { data: result, ts: Date.now() });
      setData(result);
    } catch (err) {
      setError(extractError(err, errorMessage));
      console.error(`${errorMessage}:`, err);
    } finally {
      setIsLoading(false);
    }
  }, [key, fetchCached, minRefetchMs, errorMessage]);

  useEffect(() => {
    let cancelled = false;
    fetchCached()
      .then((d) => {
        if (!cancelled) {
          setData(d);
          setError(null);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setError(extractError(err, errorMessage));
          console.error(`${errorMessage}:`, err);
        }
      })
      .finally(() => {
        if (!cancelled) setIsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [fetchCached, errorMessage]);

  return { data, isLoading, error, refetch };
}
