import { useState, useEffect, useRef } from 'react';
import { SectionCard } from './common';
import { useEventStream } from '../../hooks/useEventStream';
import { EVENTS_URL } from '../../services/api';

export function LogSection() {
  const [logs, setLogs] = useState<string[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const pendingLogs = useRef<string[]>([]);
  const rafId = useRef<number>(0);

  useEventStream(EVENTS_URL, (event) => {
    const timestamp = new Date().toLocaleTimeString();
    const logLine = `[${timestamp}] ${event.type}: ${JSON.stringify(event.data).slice(0, 120)}`;
    pendingLogs.current.push(logLine);
    if (!rafId.current) {
      rafId.current = requestAnimationFrame(() => {
        const batch = pendingLogs.current;
        pendingLogs.current = [];
        rafId.current = 0;
        setLogs(prev => [...prev, ...batch].slice(-100));
      });
    }
  });

  useEffect(() => {
    return () => { if (rafId.current) cancelAnimationFrame(rafId.current); };
  }, []);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [logs]);

  return (
    <SectionCard title="Event Log">
      <div ref={scrollRef} className="h-[60vh] overflow-y-auto font-mono text-[10px] space-y-1 no-scrollbar">
        {logs.length === 0 && <div className="opacity-30">AWAITING_SIGNAL...</div>}
        {logs.map((log, i) => (
          <div key={i} className="text-content-secondary animate-in fade-in slide-in-from-left-1 duration-300">
            <span className="opacity-50 mr-2">&gt;</span>{log}
          </div>
        ))}
      </div>
    </SectionCard>
  );
}
