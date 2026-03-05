import { useState, useRef, useEffect, useCallback } from 'react';
import { createPortal } from 'react-dom';
import { Zap, ChevronDown } from 'lucide-react';
import { McpServerInfo } from '../types';

interface EngineSelectorProps {
  servers: McpServerInfo[];
  selectedEngine: string | null;
  onSelect: (engineId: string | null) => void;
  disabled?: boolean;
}

function resolveDisplayName(server: McpServerInfo): string {
  if (server.display_name) return server.display_name;
  const shortId = server.id.replace('mind.', '');
  return shortId.charAt(0).toUpperCase() + shortId.slice(1);
}

export function EngineSelector({ servers, selectedEngine, onSelect, disabled }: EngineSelectorProps) {
  const [isOpen, setIsOpen] = useState(false);
  const btnRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; bottom: number; maxHeight: number } | null>(null);

  const mindServers = servers.filter(s => s.id.startsWith('mind.'));
  const selected = selectedEngine
    ? mindServers.find(s => s.id === selectedEngine)
    : null;

  const pillLabel = selected ? resolveDisplayName(selected) : 'Auto';

  // Compute position when opening — anchor above the entire input bar
  const open = useCallback(() => {
    if (!btnRef.current) return;
    const bar = btnRef.current.closest('.border-t') as HTMLElement | null;
    const anchorTop = bar ? bar.getBoundingClientRect().top : btnRef.current.getBoundingClientRect().top;
    const bottomOffset = window.innerHeight - anchorTop + 6;
    const maxH = Math.min(320, anchorTop - 12);
    setPos({
      left: btnRef.current.getBoundingClientRect().left,
      bottom: bottomOffset,
      maxHeight: maxH,
    });
    setIsOpen(true);
  }, []);

  // Close on outside click — blur button to prevent focus flash
  useEffect(() => {
    if (!isOpen) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as Node;
      if (
        btnRef.current?.contains(target) ||
        menuRef.current?.contains(target)
      ) return;
      setIsOpen(false);
      btnRef.current?.blur();
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [isOpen]);

  return (
    <>
      {/* Pill button */}
      <button
        ref={btnRef}
        onMouseDown={e => e.preventDefault()}
        onClick={() => { if (disabled) return; if (isOpen) { setIsOpen(false); btnRef.current?.blur(); } else { open(); } }}
        disabled={disabled}
        className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[10px] font-mono font-bold uppercase tracking-wider border transition-colors disabled:opacity-30 ${
          selectedEngine
            ? 'border-brand/40 bg-brand/10 text-brand'
            : 'border-edge bg-glass text-content-secondary hover:text-brand hover:border-brand/30'
        }`}
        style={{ outline: 'none' }}
        title="Select engine"
      >
        <Zap size={12} />
        <span>{pillLabel}</span>
        <ChevronDown size={10} className={`transition-transform ${isOpen ? 'rotate-180' : ''}`} />
      </button>

      {/* Popover menu — portaled to body */}
      {isOpen && pos && createPortal(
        <div
          ref={menuRef}
          className="fixed w-48 bg-surface-primary/95 backdrop-blur-xl border border-edge rounded-xl shadow-2xl shadow-black/40 overflow-y-auto py-1 z-[9998]"
          style={{ left: pos.left, bottom: pos.bottom, maxHeight: pos.maxHeight }}
        >
          {/* Auto option */}
          <button
            onMouseDown={e => e.preventDefault()}
            onClick={() => { onSelect(null); setIsOpen(false); btnRef.current?.blur(); }}
            className={`no-focus-ring w-full flex items-center gap-2 px-3 py-2.5 text-[11px] font-mono transition-colors ${
              !selectedEngine
                ? 'bg-brand/10 text-brand'
                : 'text-content-secondary hover:bg-glass hover:text-content-primary'
            }`}
          >
            <Zap size={12} className="text-amber-400" />
            <span className="font-bold">Auto</span>
            <span className="ml-auto text-[9px] text-content-tertiary">CFR</span>
          </button>

          {/* Divider */}
          {mindServers.length > 0 && (
            <div className="border-t border-edge" />
          )}

          {/* Engine list */}
          {mindServers.map(server => {
            const isConnected = server.status === 'Connected';
            const isSelected = selectedEngine === server.id;
            return (
              <button
                key={server.id}
                onMouseDown={e => e.preventDefault()}
                onClick={() => { onSelect(server.id); setIsOpen(false); btnRef.current?.blur(); }}
                className={`no-focus-ring w-full flex items-center gap-2 px-3 py-2 text-[11px] font-mono transition-colors ${
                  isSelected
                    ? 'bg-brand/10 text-brand'
                    : isConnected
                      ? 'text-content-secondary hover:bg-glass hover:text-content-primary'
                      : 'text-content-muted'
                }`}
              >
                <span className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${
                  isConnected ? 'bg-emerald-400' : 'bg-neutral-600'
                }`} />
                <span className={!isConnected ? 'opacity-50' : ''}>
                  {resolveDisplayName(server)}
                </span>
                {!isConnected && (
                  <span className="ml-auto text-[9px] text-content-muted">offline</span>
                )}
              </button>
            );
          })}
        </div>,
        document.body
      )}
    </>
  );
}
