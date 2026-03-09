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

function connect(url: string, apiKey?: string) {
  if (sharedEventSource && sharedEventSource.readyState !== EventSource.CLOSED) {
    return; // Already connected
  }

  sharedUrl = url;
  sharedApiKey = apiKey ?? null;
  // SSE EventSource cannot set custom headers — pass token via query param (bug-157)
  const connectUrl = apiKey ? `${url}${url.includes('?') ? '&' : '?'}token=${encodeURIComponent(apiKey)}` : url;
  console.log(`📡 Connecting to Event Stream: ${url}`);
  const es = new EventSource(connectUrl);
  sharedEventSource = es;

  es.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data);
      attempt = 0; // Reset backoff on successful message
      subscribers.forEach((handler) => handler(data));
    } catch (err) {
      console.error('Failed to parse SSE event:', err);
    }
  };

  es.onerror = () => {
    const delay = Math.min(INITIAL_DELAY_MS * Math.pow(2, attempt), MAX_DELAY_MS);
    attempt++;
    console.error(`SSE Connection Error. Retrying in ${delay / 1000}s...`);
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
}

export function useEventStream(
  url: string,
  onMessage: (data: ServerEvent) => void,
  apiKey?: string,
) {
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
