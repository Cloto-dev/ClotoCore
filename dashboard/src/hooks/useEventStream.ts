import { useEffect, useRef } from 'react';

// M-22: Exponential backoff constants
const INITIAL_DELAY_MS = 5000;
const MAX_DELAY_MS = 30000;

export function useEventStream(
  url: string,
  onMessage: (data: any) => void
) {
  const reconnectTimeout = useRef<number | null>(null);
  const eventSourceRef = useRef<EventSource | null>(null);
  const onMessageRef = useRef(onMessage);
  const attemptRef = useRef(0);

  useEffect(() => {
    onMessageRef.current = onMessage;
  }, [onMessage]);

  useEffect(() => {
    const connect = () => {
      console.log(`📡 Connecting to Event Stream: ${url}`);
      const es = new EventSource(url);
      eventSourceRef.current = es;

      es.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data);
          attemptRef.current = 0; // M-22: Reset backoff on successful message
          if (onMessageRef.current) {
            onMessageRef.current(data);
          }
        } catch (err) {
          console.error("Failed to parse SSE event:", err);
        }
      };

      es.onerror = (err) => {
        // M-22: Exponential backoff (5s, 10s, 20s, 30s max)
        const delay = Math.min(INITIAL_DELAY_MS * Math.pow(2, attemptRef.current), MAX_DELAY_MS);
        attemptRef.current++;
        console.error(`SSE Connection Error. Retrying in ${delay / 1000}s...`, err);
        es.close();
        if (reconnectTimeout.current) clearTimeout(reconnectTimeout.current);
        reconnectTimeout.current = window.setTimeout(connect, delay);
      };
    };

    connect();

    return () => {
      if (eventSourceRef.current) eventSourceRef.current.close();
      if (reconnectTimeout.current) clearTimeout(reconnectTimeout.current);
    };
  }, [url]);
}
