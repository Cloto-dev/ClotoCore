import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { displayServerId } from '../lib/format';
import { isEngineServer } from '../lib/serverCategory';
import type { McpServerInfo } from '../types';

interface EngineSelectorProps {
  servers: McpServerInfo[];
  selectedEngine: string | null;
  onSelect: (engineId: string | null) => void;
  disabled?: boolean;
}

function resolveDisplayName(server: McpServerInfo): string {
  if (server.display_name) return server.display_name;
  const shortId = displayServerId(server.id);
  return shortId.charAt(0).toUpperCase() + shortId.slice(1);
}

/**
 * The engine choice in the composer's row (docs/gui/samples/02-chat-conversation.html):
 * a text button reading "Auto" or the engine's name, and a menu above it.
 */
export function EngineSelector({ servers, selectedEngine, onSelect, disabled }: EngineSelectorProps) {
  const { t } = useTranslation('agents');
  const [isOpen, setIsOpen] = useState(false);
  const anchorRef = useRef<HTMLDivElement>(null);

  const mindServers = servers.filter(isEngineServer);
  const selected = selectedEngine ? mindServers.find((s) => s.id === selectedEngine) : null;
  const label = selected ? resolveDisplayName(selected) : t('chat_input.engine_auto');

  useEffect(() => {
    if (!isOpen) return;
    const onDown = (e: MouseEvent) => {
      if (!anchorRef.current?.contains(e.target as Node)) setIsOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setIsOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [isOpen]);

  const choose = (id: string | null) => {
    onSelect(id);
    setIsOpen(false);
  };

  return (
    <div className="menu-anchor" ref={anchorRef}>
      <button
        type="button"
        className="ctx"
        onClick={() => !disabled && setIsOpen((v) => !v)}
        disabled={disabled}
        title={t('chat_input.engine', { name: label })}
        aria-label={t('chat_input.engine', { name: label })}
        aria-haspopup="menu"
        aria-expanded={isOpen}
      >
        {label}
      </button>
      {isOpen && (
        <div className="menu up" role="menu">
          <button type="button" role="menuitem" className={!selectedEngine ? 'on' : ''} onClick={() => choose(null)}>
            {t('chat_input.engine_auto')}
          </button>
          {mindServers.map((server) => {
            const isConnected = server.status === 'Connected';
            return (
              <button
                type="button"
                role="menuitem"
                key={server.id}
                className={selectedEngine === server.id ? 'on' : ''}
                onClick={() => choose(server.id)}
              >
                {resolveDisplayName(server)}
                {!isConnected && <span className="dim">{t('chat_input.engine_offline')}</span>}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
