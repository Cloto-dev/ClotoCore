import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { useEventStream } from '../../hooks/useEventStream';
import { EVENTS_URL } from '../../services/api';
import { SettingsGroup } from './common';

export function LogSection() {
  const api = useApi();
  const { t } = useTranslation('settings');
  const [logs, setLogs] = useState<string[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const pendingLogs = useRef<string[]>([]);
  const rafId = useRef<number>(0);

  useEventStream(
    EVENTS_URL,
    (event) => {
      const timestamp = new Date().toLocaleTimeString();
      const logLine = `[${timestamp}] ${event.type}: ${JSON.stringify(event.data).slice(0, 120)}`;
      pendingLogs.current.push(logLine);
      if (!rafId.current) {
        rafId.current = requestAnimationFrame(() => {
          const batch = pendingLogs.current;
          pendingLogs.current = [];
          rafId.current = 0;
          setLogs((prev) => [...prev, ...batch].slice(-100));
        });
      }
    },
    api.apiKey,
  );

  useEffect(() => {
    return () => {
      if (rafId.current) cancelAnimationFrame(rafId.current);
    };
  }, []);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, []);

  return (
    <SettingsGroup title={t('log.title')}>
      <div className="set-block">
        <div ref={scrollRef} className="set-log">
          {logs.length === 0 && <div className="waiting">{t('log.awaiting_signal')}</div>}
          {logs.map((log, i) => (
            <div key={i}>
              <span className="t">&gt;</span>
              {log}
            </div>
          ))}
        </div>
      </div>
    </SettingsGroup>
  );
}
