import { useState, useEffect, useCallback, useRef } from 'react';
import { useEventStream } from './useEventStream';
import { API_BASE } from '../services/api';
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

interface ThoughtLine {
  id: number;
  text: string;
  timestamp: number;
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

    if (data.type === "Thought") {
      setThoughtLines(prev => {
        const text = data.payload?.content || JSON.stringify(data.data) || "Unknown Thought";
        const next = [...prev, { id: nextId.current++, text, timestamp: eventTimestamp }];
        return next.slice(-12);
      });
    } else if (data.type === "ResponseGenerated") {
      const text = (data.payload?.content || "").replace(/\n/g, ' ');
      const segments: string[] = text.match(/.{1,100}/g) || [];
      setThoughtLines(prev => {
        const next = [...prev, ...segments.slice(0, 3).map((s: string) => ({ id: nextId.current++, text: s, timestamp: eventTimestamp }))];
        return next.slice(-12);
      });
    } else {
      // H-17: Cap event history to prevent unbounded memory growth
      setEventHistory(prev => [...prev, { ...data, timestamp: eventTimestamp }].slice(-500));
      if (data.type === "ToolEnd" || data.type === "MessageReceived") fetchMetrics();

      // Forward SystemNotification events to OS notifications (Tauri only)
      if (data.type === "SystemNotification" && isTauri) {
        const message = typeof data.data === 'string' ? data.data : JSON.stringify(data.data);
        sendNativeNotification("Exiv System", message);
      }
    }
  }, [fetchMetrics, isHistoryLoaded]);

  useEventStream(`${API_BASE}/events`, handleEvent);

  useEffect(() => {
    const fetchHistory = async () => {
      try {
        const res = await fetch(`${API_BASE}/history`);
        if (res.ok) {
          const history: StrictSystemEvent[] = await res.json();
          const now = Date.now();
          
          setEventHistory(history.map(e => ({ ...e, timestamp: e.timestamp || now })));
          
          const historicalThoughts: ThoughtLine[] = [];
          history.forEach(e => {
            const eventTimestamp = e.timestamp || now;
            // Skip thoughts older than 30 seconds
            if (now - eventTimestamp > 30000) return;

            if (e.type === "Thought") {
              historicalThoughts.push({ id: nextId.current++, text: e.payload.content, timestamp: eventTimestamp });
            } else if (e.type === "ResponseGenerated") {
              const text = e.payload.content.replace(/\n/g, ' ');
              const segments: string[] = text.match(/.{1,100}/g) || [];
              segments.slice(0, 3).forEach(s => historicalThoughts.push({ id: nextId.current++, text: s, timestamp: eventTimestamp }));
            }
          });
          setThoughtLines(historicalThoughts.slice(-12));
          
          // Process pending events
          setIsHistoryLoaded(true);
          pendingEvents.current.forEach(handleEvent);
          pendingEvents.current = [];
        }
      } catch (e) {
        console.error("Failed to fetch history", e);
      }
    };
    fetchHistory();
  }, [fetchMetrics, handleEvent]);

  return { eventHistory, thoughtLines, isHistoryLoaded };
};
