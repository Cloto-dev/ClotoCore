import { useEffect, useRef } from 'react';
import { isTauri } from '../lib/tauri';
import { browserSessionReady } from '../services/session';

// Singleton SSE connection shared across all consumers
// Prevents multiple EventSource instances to the same endpoint

type ServerEvent = { type: string; data: Record<string, unknown>; [key: string]: unknown };
type Handler = (data: ServerEvent) => void;

// Exponential backoff constants
const INITIAL_DELAY_MS = 5000;
const MAX_DELAY_MS = 30000;

// Module-level shared state
let sharedEventSource: EventSource | null = null;
let sharedUrl: string | null = null;
const subscribers = new Set<Handler>();
let reconnectTimeout: number | null = null;
let attempt = 0;

let sharedApiKey: string | null = null;
let hasConnectedBefore = false;
let lastSeenSeqId = 0;

/**
 * The credential this connection puts in the URL, if any.
 *
 * `EventSource` cannot set headers (bug-157), so something has to travel with
 * the URL — unless the browser is already carrying a cookie for this origin,
 * which it attaches to the stream by itself. In a browser that is the case
 * (`services/session`), so the URL stays clean; under Tauri the SPA's origin is
 * not the API's, no cookie is ever sent, and the query parameter is still the
 * only channel.
 */
function urlToken(apiKey?: string): string | undefined {
  return isTauri ? apiKey : undefined;
}

function connect(url: string, apiKey?: string) {
  if (sharedEventSource && sharedEventSource.readyState !== EventSource.CLOSED) {
    return; // Already connected
  }

  sharedUrl = url;
  sharedApiKey = apiKey ?? null;
  const token = urlToken(apiKey);
  const connectUrl = token ? `${url}${url.includes('?') ? '&' : '?'}token=${encodeURIComponent(token)}` : url;
  if (import.meta.env.DEV) console.log(`📡 Connecting to Event Stream: ${url}`);
  const es = new EventSource(connectUrl);
  sharedEventSource = es;

  es.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data);
      // Dedup by seq_id (prevents replayed events from being processed twice)
      if (event.lastEventId) {
        const seqId = parseInt(event.lastEventId, 10);
        if (!Number.isNaN(seqId)) {
          if (seqId <= lastSeenSeqId) return;
          lastSeenSeqId = seqId;
        }
      }
      attempt = 0; // Reset backoff on successful message
      subscribers.forEach((handler) => handler(data));
    } catch (err) {
      if (import.meta.env.DEV) console.error('Failed to parse SSE event:', err);
    }
  };

  es.addEventListener('handshake', () => {
    attempt = 0;
    if (hasConnectedBefore) {
      // Notify consumers of reconnection so they can refetch missed data
      subscribers.forEach((h) => h({ type: '__reconnected', data: {} } as ServerEvent));
    }
    hasConnectedBefore = true;
  });

  es.addEventListener('lagged', (e: MessageEvent) => {
    const count = parseInt(e.data, 10) || 0;
    if (import.meta.env.DEV) console.warn(`SSE: Server lagged by ${count} messages`);
    subscribers.forEach((h) => h({ type: '__lagged', data: { count } } as ServerEvent));
  });

  es.onerror = () => {
    const delay = Math.min(INITIAL_DELAY_MS * 2 ** attempt, MAX_DELAY_MS);
    attempt++;
    if (import.meta.env.DEV) console.error(`SSE Connection Error. Retrying in ${delay / 1000}s...`);
    es.close();
    sharedEventSource = null;
    if (reconnectTimeout) clearTimeout(reconnectTimeout);
    reconnectTimeout = window.setTimeout(() => {
      if (subscribers.size > 0 && sharedUrl) {
        connect(sharedUrl, sharedApiKey ?? undefined);
      }
    }, delay);
  };
}

function disconnect() {
  if (subscribers.size > 0) return; // Other consumers still active
  if (sharedEventSource) {
    sharedEventSource.close();
    sharedEventSource = null;
  }
  if (reconnectTimeout) {
    clearTimeout(reconnectTimeout);
    reconnectTimeout = null;
  }
  sharedUrl = null;
  attempt = 0;
  hasConnectedBefore = false;
  lastSeenSeqId = 0;
}

export function useEventStream(url: string, onMessage: (data: ServerEvent) => void, apiKey?: string) {
  const handlerRef = useRef(onMessage);

  useEffect(() => {
    handlerRef.current = onMessage;
  }, [onMessage]);

  useEffect(() => {
    const handler: Handler = (data) => handlerRef.current(data);
    subscribers.add(handler);

    // In a browser the cookie has to be in place before the stream is opened: a
    // refused EventSource does not retry promptly, it starts backing off, so a
    // race here would cost seconds of missing events rather than a retry.
    // `false` means no key has been offered yet — there is nothing to connect
    // with, and this effect re-runs when one arrives.
    let cancelled = false;
    browserSessionReady().then((ready) => {
      if (cancelled || !ready) return;
      connect(url, apiKey);
    });

    return () => {
      cancelled = true;
      subscribers.delete(handler);
      disconnect();
    };
  }, [url, apiKey]);
}
