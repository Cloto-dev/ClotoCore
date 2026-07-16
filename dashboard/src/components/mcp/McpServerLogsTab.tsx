import { Trash2 } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { useEventStream } from '../../hooks/useEventStream';
import { displayServerId } from '../../lib/format';
import { EVENTS_URL } from '../../services/api';
import type { McpServerInfo } from '../../types';

// Where a log line originated (kernel `McpLogSource`, snake_case on the wire).
type LogSource = 'stderr' | 'mcp_logging';

// Wire shape of a `ClotoEventData::McpServerLog` event's `data` (see
// docs/MCP_SERVER_LOGS_DESIGN.md §5). Fields live under `event.data` per the
// adjacent-tag serde contract — reading `event.payload` was bug-423.
interface McpServerLogData {
  server_id: string;
  source: LogSource;
  level?: string;
  logger?: string;
  message: string;
  timestamp?: string;
}

interface LogEntry {
  timestamp: string;
  // Present for McpServerLog lines; absent for plain MGP notifications.
  source?: LogSource;
  level?: string;
  logger?: string;
  // Badge text: 'stderr' / 'MCP' for logs, or the notification method.
  label: string;
  message: string;
}

interface Props {
  server: McpServerInfo;
}

// Severity → text color for the level badge (RFC 5424 order). Only the classes
// below are used, so they stay in the pre-compiled Tailwind bundle.
const LEVEL_COLOR: Record<string, string> = {
  debug: 'text-content-tertiary',
  info: 'text-brand',
  notice: 'text-brand',
  warning: 'text-amber-500',
  error: 'text-red-500',
  critical: 'text-red-500',
  alert: 'text-red-500',
  emergency: 'text-red-500',
};

function formatTimestamp(value: unknown): string {
  const date = new Date((value as string | number | undefined) ?? Date.now());
  return Number.isNaN(date.getTime()) ? new Date().toISOString().slice(11, 19) : date.toISOString().slice(11, 19);
}

// Map a kernel event (live SSE or history) to a log entry for this server, or
// null if it is not a log-bearing event for it. Kernel events carry their
// fields under `event.data` (adjacent-tag serde contract) and the discriminant
// is mixed-case `Mcp…` — reading `event.payload` / matching `includes('MCP')`
// were bug-423 / bug-424.
function eventToLogEntry(
  event: { type?: string; data?: unknown; timestamp?: unknown },
  serverId: string,
): LogEntry | null {
  const data = (event.data ?? {}) as Record<string, unknown>;
  if (data.server_id !== serverId) return null;

  if (event.type === 'McpServerLog') {
    const log = data as unknown as McpServerLogData;
    return {
      timestamp: formatTimestamp(log.timestamp ?? event.timestamp),
      source: log.source,
      level: log.level,
      logger: log.logger,
      label: log.source === 'mcp_logging' ? 'MCP' : 'stderr',
      message: String(log.message ?? ''),
    };
  }
  if (event.type === 'McpNotification') {
    // MGP notifications are surfaced too, without a source/level badge.
    return {
      timestamp: formatTimestamp(event.timestamp),
      label: String((data.method as string | undefined) ?? 'notify'),
      message: JSON.stringify(data.params ?? {}).slice(0, 200),
    };
  }
  return null;
}

const MAX_LOG_LINES = 200;

export function McpServerLogsTab({ server }: Props) {
  const api = useApi();
  const { t } = useTranslation('mcp');
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const bottomRef = useRef<HTMLDivElement>(null);

  const handleEvent = useCallback(
    (event: any) => {
      const entry = eventToLogEntry(event, server.id);
      if (entry) setLogs((prev) => [...prev.slice(-(MAX_LOG_LINES - 1)), entry]);
    },
    [server.id],
  );

  useEventStream(EVENTS_URL, handleEvent, api.apiKey);

  // Seed from the kernel event-history ring buffer so opening the tab shows the
  // server's recent logs immediately, instead of an empty "waiting" state until
  // the next line arrives (the SSE stream only live-tails from mount and does
  // not replay history on a fresh connection). See MCP_SERVER_LOGS_DESIGN.md §7.
  const getHistory = api.getHistory;
  useEffect(() => {
    let cancelled = false;
    getHistory()
      .then((events) => {
        if (cancelled) return;
        const seeded = (events ?? [])
          .map((e) => eventToLogEntry(e, server.id))
          .filter((e): e is LogEntry => e !== null)
          .slice(-MAX_LOG_LINES);
        // Prepend seed only if live events haven't already populated the list,
        // then cap — avoids clobbering lines that arrived during the fetch.
        // bug-481: the history snapshot and the live SSE tail overlap for
        // events emitted around mount, so drop seed entries already present in
        // prev (keyed by timestamp+label+message) to avoid duplicate lines.
        setLogs((prev) => {
          if (prev.length === 0) return seeded;
          const keyOf = (e: LogEntry) => `${e.timestamp}|${e.label}|${e.message}`;
          const live = new Set(prev.map(keyOf));
          const fresh = seeded.filter((e) => !live.has(keyOf(e)));
          return [...fresh, ...prev].slice(-MAX_LOG_LINES);
        });
      })
      .catch(() => {
        /* history may be unavailable */
      });
    return () => {
      cancelled = true;
    };
  }, [server.id, getHistory]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, []);

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2 border-b border-edge">
        <span className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary">
          {t('logs.event_log')} — {displayServerId(server.id)}
        </span>
        <button
          onClick={() => setLogs([])}
          className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors"
          title={t('logs.clear')}
          aria-label={t('logs.clear')}
        >
          <Trash2 size={12} />
        </button>
      </div>

      {/* Log entries */}
      <div className="flex-1 overflow-y-auto p-2 font-mono text-[10px] bg-black/5 dark:bg-white/5">
        {logs.length === 0 && <div className="text-content-tertiary text-center py-8">{t('logs.waiting')}</div>}
        {logs.map((log, i) => (
          <div
            key={`${log.timestamp}-${log.label}-${i}`}
            className="flex items-baseline gap-2 py-0.5 hover:bg-glass rounded px-1"
          >
            <span className="text-content-tertiary flex-shrink-0">{log.timestamp}</span>
            {/* Source badge — the required stderr vs MCP-logging distinction. */}
            <span
              className={`flex-shrink-0 px-1 rounded bg-glass ${
                log.source === 'mcp_logging' ? 'text-brand' : 'text-content-tertiary'
              }`}
            >
              {log.label}
            </span>
            {/* Level badge — MCP-logging lines only. */}
            {log.level && (
              <span
                className={`flex-shrink-0 uppercase text-[9px] ${LEVEL_COLOR[log.level] ?? 'text-content-tertiary'}`}
              >
                {log.level}
              </span>
            )}
            {log.logger && <span className="text-content-tertiary flex-shrink-0">{log.logger}</span>}
            <span className="text-content-secondary truncate">{log.message}</span>
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
