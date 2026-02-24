import { useState, useEffect, useCallback, useRef } from 'react';
import { useEventStream } from './useEventStream';
import { api, EVENTS_URL } from '../services/api';
import type { StrictSystemEvent } from '../types';
import { isTauri } from '../lib/tauri';

/** Send an OS notification in Tauri mode (no-op in browser). */
async function sendNativeNotification(title: string, body: string) {
  if (!isTauri) return;
  try {
    const { isPermissionGranted, requestPermission, sendNotification } =
      await import('@tauri-apps/plugin-notification');
    let permitted = await isPermissionGranted();
    if (!permitted) {
      const result = await requestPermission();
      permitted = result === 'granted';
    }
    if (permitted) {
      sendNotification({ title, body });
    }
  } catch {
    // Notification plugin not available or permission denied - silently skip
  }
}

export interface ThoughtLine {
  id: number;
  text: string;
  timestamp: number;
}

function eventToThoughtLines(
  e: StrictSystemEvent,
  eventTimestamp: number,
  nextIdRef: React.MutableRefObject<number>,
): ThoughtLine[] {
  if (e.type === "Thought") {
    const text = e.payload?.content || JSON.stringify(e.data) || "Unknown Thought";
    return [{ id: nextIdRef.current++, text, timestamp: eventTimestamp }];
  }
  if (e.type === "ResponseGenerated") {
    const text = (e.payload?.content || "").replace(/\n/g, ' ');
    const segments: string[] = text.match(/.{1,100}/g) || [];
    return segments.slice(0, 3).map(s => ({ id: nextIdRef.current++, text: s, timestamp: eventTimestamp }));
  }
  return [];
}

export const useStatusManager = (fetchMetrics: () => void) => {
  const [eventHistory, setEventHistory] = useState<StrictSystemEvent[]>([]);
  const [thoughtLines, setThoughtLines] = useState<ThoughtLine[]>([]);
  const [isHistoryLoaded, setIsHistoryLoaded] = useState(false);
  const nextId = useRef(0);
  const pendingEvents = useRef<StrictSystemEvent[]>([]);

  useEffect(() => {
    const interval = setInterval(() => {
      const now = Date.now();
      setThoughtLines(prev => prev.filter(line => now - line.timestamp < 30000));
    }, 100);
    return () => clearInterval(interval);
  }, []);

  const handleEvent = useCallback((data: StrictSystemEvent) => {
    if (!isHistoryLoaded) {
      pendingEvents.current.push(data);
      return;
    }

    const eventTimestamp = data.timestamp || Date.now();

    const lines = eventToThoughtLines(data, eventTimestamp, nextId);
    if (lines.length > 0) {
      setThoughtLines(prev => [...prev, ...lines].slice(-12));
    } else {
      // H-17: Cap event history to prevent unbounded memory growth
      setEventHistory(prev => [...prev, { ...data, timestamp: eventTimestamp }].slice(-500));
      if (data.type === "ToolEnd" || data.type === "MessageReceived") fetchMetrics();

      // Forward SystemNotification events to OS notifications (Tauri only)
      if (data.type === "SystemNotification" && isTauri) {
        const message = typeof data.data === 'string' ? data.data : JSON.stringify(data.data);
        sendNativeNotification("Cloto System", message);
      }
    }
  }, [fetchMetrics, isHistoryLoaded]);

  useEventStream(EVENTS_URL, handleEvent);

  useEffect(() => {
    const fetchHistory = async () => {
      try {
        const history: StrictSystemEvent[] = await api.getHistory();
        const now = Date.now();

        setEventHistory(history.map(e => ({ ...e, timestamp: e.timestamp || now })));

        const historicalThoughts: ThoughtLine[] = [];
        history.forEach(e => {
          const eventTimestamp = e.timestamp || now;
          if (now - eventTimestamp > 30000) return;
          historicalThoughts.push(...eventToThoughtLines(e, eventTimestamp, nextId));
        });
        setThoughtLines(historicalThoughts.slice(-12));

        // Process pending events
        setIsHistoryLoaded(true);
        pendingEvents.current.forEach(handleEvent);
        pendingEvents.current = [];
      } catch (e) {
        console.error("Failed to fetch history", e);
      }
    };
    fetchHistory();
  }, [fetchMetrics, handleEvent]);

  return { eventHistory, thoughtLines, isHistoryLoaded };
};
