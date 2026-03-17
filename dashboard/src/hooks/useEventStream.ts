import { useEffect, useRef } from 'react';

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

function connect(url: string, apiKey?: string) {
  if (sharedEventSource && sharedEventSource.readyState !== EventSource.CLOSED) {
    return; // Already connected
  }

  sharedUrl = url;
  sharedApiKey = apiKey ?? null;
  // SSE EventSource cannot set custom headers — pass token via query param (bug-157)
  const connectUrl = apiKey ? `${url}${url.includes('?') ? '&' : '?'}token=${encodeURIComponent(apiKey)}` : url;
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
    connect(url, apiKey);

    return () => {
      subscribers.delete(handler);
      disconnect();
    };
  }, [url, apiKey]);
}
