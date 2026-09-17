import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { useEventStream } from '../../hooks/useEventStream';
import { EVENTS_URL } from '../../services/api';

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

function formatTimestamp(value: unknown): string {
  const date = new Date((value as string | number | undefined) ?? Date.now());
  return Number.isNaN(date.getTime()) ? new Date().toISOString().slice(11, 19) : date.toISOString().slice(11, 19);
}

// Map a kernel event (live SSE or history) to a log entry for this server, or
// null if it is not a log-bearing event for it. Kernel events carry their
// fields under `event.data` (adjacent-tag serde contract) and the discriminant
// is mixed-case `Mcp…` — reading `event.payload` / matching `includes('MCP')`
// were bug-423 / bug-424.
export function eventToLogEntry(
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

/** The most recent lines kept on screen (docs/gui/samples/07-mcp-server-detail.html). */
export const MAX_LOG_LINES = 200;

const WARN_LEVELS = new Set(['warning']);
const ERROR_LEVELS = new Set(['error', 'critical', 'alert', 'emergency']);

/** The server's recent log lines, as the mock's `.log` block. */
export function McpLogsSection({ serverId }: { serverId: string }) {
  const api = useApi();
  const { t } = useTranslation('mcp');
  const [logs, setLogs] = useState<LogEntry[]>([]);

  const handleEvent = useCallback(
    (event: { type?: string; data?: unknown; timestamp?: unknown }) => {
      const entry = eventToLogEntry(event, serverId);
      if (entry) setLogs((prev) => [...prev.slice(-(MAX_LOG_LINES - 1)), entry]);
    },
    [serverId],
  );

  useEventStream(EVENTS_URL, handleEvent, api.apiKey);

  // Seed from the kernel event-history ring buffer so opening the section shows
  // the server's recent logs immediately, instead of an empty "waiting" state
  // until the next line arrives (the SSE stream only live-tails from mount and
  // does not replay history on a fresh connection). See MCP_SERVER_LOGS_DESIGN.md §7.
  const getHistory = api.getHistory;
  useEffect(() => {
    let cancelled = false;
    getHistory()
      .then((events) => {
        if (cancelled) return;
        const seeded = (events ?? [])
          .map((e) => eventToLogEntry(e, serverId))
          .filter((e): e is LogEntry => e !== null)
          .slice(-MAX_LOG_LINES);
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
  }, [serverId, getHistory]);

  return (
    <>
      {logs.length === 0 ? (
        <div className="hint">{t('logs.waiting')}</div>
      ) : (
        <div className="log">
          {logs.map((log, i) => {
            const level = log.level?.toLowerCase();
            const cls = level && ERROR_LEVELS.has(level) ? 'e' : level && WARN_LEVELS.has(level) ? 'w' : '';
            return (
              <div key={`${log.timestamp}-${log.label}-${i}`}>
                <span className="t">{log.timestamp}</span> {log.label}
                {level && (
                  <>
                    {' '}
                    <span className={cls}>{level}</span>
                  </>
                )}
                {log.logger && ` ${log.logger}`} {log.message}
              </div>
            );
          })}
        </div>
      )}
      <div className="hint">{t('logs.recent', { count: MAX_LOG_LINES })}</div>
    </>
  );
}
