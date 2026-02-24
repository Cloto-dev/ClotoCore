import { useState, useEffect, useCallback, useRef } from 'react';
import { api, EVENTS_URL } from '../services/api';
import { useEventStream } from './useEventStream';
import type {
  EvolutionStatus,
  GenerationRecord,
  FitnessLogEntry,
  RollbackRecord,
  EvolutionEvent,
} from '../types';

const MAX_EVENTS = 50;
const DEBOUNCE_MS = 500;
const POLL_INTERVAL_MS = 30000;

export function useEvolution() {
  const [status, setStatus] = useState<EvolutionStatus | null>(null);
  const [timeline, setTimeline] = useState<FitnessLogEntry[]>([]);
  const [generations, setGenerations] = useState<GenerationRecord[]>([]);
  const [rollbacks, setRollbacks] = useState<RollbackRecord[]>([]);
  const [events, setEvents] = useState<EvolutionEvent[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const fetchAll = useCallback(async () => {
    try {
      const [s, t, g, r] = await Promise.all([
        api.getEvolutionStatus(),
        api.getFitnessTimeline(200),
        api.getGenerationHistory(50),
        api.getRollbackHistory(),
      ]);
      setStatus(s);
      setTimeline(t);
      setGenerations(g);
      setRollbacks(r);
      setError(null);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : 'Failed to load evolution data');
    } finally {
      setLoading(false);
    }
  }, []);

  const debouncedRefresh = useCallback(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = window.setTimeout(() => {
      fetchAll();
    }, DEBOUNCE_MS);
  }, [fetchAll]);

  // SSE: filter evolution events and trigger refresh
  useEventStream(EVENTS_URL, useCallback((data: any) => {
    const type: string = data?.type || '';
    if (!type.startsWith('Evolution')) return;

    const evt: EvolutionEvent = {
      type,
      data: data?.data || data,
      timestamp: data?.timestamp || Date.now(),
    };
    setEvents(prev => [evt, ...prev].slice(0, MAX_EVENTS));
    debouncedRefresh();
  }, [debouncedRefresh]));

  // Initial load
  useEffect(() => {
    fetchAll();
  }, [fetchAll]);

  // Polling fallback
  useEffect(() => {
    const interval = setInterval(fetchAll, POLL_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [fetchAll]);

  // Cleanup debounce on unmount
  useEffect(() => {
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, []);

  return { status, timeline, generations, rollbacks, events, loading, error, refresh: fetchAll };
}
