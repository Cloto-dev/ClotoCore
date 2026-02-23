import { useState, useEffect, useRef, useCallback } from 'react';
import { McpServerInfo } from '../../types';
import { useEventStream } from '../../hooks/useEventStream';
import { EVENTS_URL } from '../../services/api';
import { Trash2 } from 'lucide-react';

interface LogEntry {
  timestamp: string;
  type: string;
  message: string;
}

interface Props {
  server: McpServerInfo;
}

export function McpServerLogsTab({ server }: Props) {
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const bottomRef = useRef<HTMLDivElement>(null);

  const handleEvent = useCallback((event: any) => {
    const payload = event.payload as Record<string, unknown> | undefined;
    const serverId = payload?.server_id as string | undefined;

    // Filter for events related to this server
    if (serverId === server.id || event.type?.includes('MCP')) {
      setLogs(prev => [...prev.slice(-199), {
        timestamp: new Date(event.timestamp ?? Date.now()).toISOString().slice(11, 19),
        type: event.type ?? 'unknown',
        message: JSON.stringify(payload ?? event.data ?? {}).slice(0, 200),
      }]);
    }
  }, [server.id]);

  useEventStream(EVENTS_URL, handleEvent);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [logs.length]);

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2 border-b border-edge">
        <span className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary">
          Event Log — {server.id}
        </span>
        <button
          onClick={() => setLogs([])}
          className="p-1 rounded hover:bg-glass text-content-muted hover:text-content-primary transition-colors"
          title="Clear"
        >
          <Trash2 size={12} />
        </button>
      </div>

      {/* Log entries */}
      <div className="flex-1 overflow-y-auto p-2 font-mono text-[10px] bg-black/5 dark:bg-white/5">
        {logs.length === 0 && (
          <div className="text-content-muted text-center py-8">
            Waiting for events...
          </div>
        )}
        {logs.map((log, i) => (
          <div key={i} className="flex gap-2 py-0.5 hover:bg-glass rounded px-1">
            <span className="text-content-muted flex-shrink-0">{log.timestamp}</span>
            <span className="text-brand flex-shrink-0">[{log.type}]</span>
            <span className="text-content-secondary truncate">{log.message}</span>
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
