import { ArrowLeft, ArrowRight, ArrowUp, HelpCircle, type LucideIcon, Minus, Square, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import { useConnection } from '../contexts/ConnectionContext';
import { closeWindow, isTauri, minimizeWindow, toggleMaximizeWindow } from '../lib/tauri';
import { StatusDot } from './ui/StatusDot';

interface ViewHeaderProps {
  icon: LucideIcon;
  title: string;
  onBack?: (() => void) | string;
  right?: React.ReactNode;
  onHelp?: () => void;
  navBack?: () => void;
  navForward?: () => void;
  canGoBack?: boolean;
  canGoForward?: boolean;
}

export function ViewHeader({
  icon: Icon,
  title,
  onBack,
  right,
  onHelp,
  navBack,
  navForward,
  canGoBack,
  canGoForward,
}: ViewHeaderProps) {
  const { connected, checking } = useConnection();
  const { t } = useTranslation();
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);

  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      setUpdateVersion(detail?.version ?? 'new');
    };
    window.addEventListener('cloto-update-available', handler);
    return () => window.removeEventListener('cloto-update-available', handler);
  }, []);

  return (
    <header
      className="relative z-10 flex items-center gap-3 px-4 py-2 border-b border-edge bg-surface-primary select-none"
      data-tauri-drag-region=""
    >
      {typeof onBack === 'string' ? (
        <Link
          to={onBack}
          className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors"
        >
          <ArrowLeft size={16} />
        </Link>
      ) : onBack ? (
        <button
          onClick={onBack}
          className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors"
        >
          <ArrowLeft size={16} />
        </button>
      ) : null}
      {navBack && (
        <div className="flex items-center gap-0.5">
          <button
            onClick={navBack}
            disabled={!canGoBack}
            className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors disabled:opacity-25 disabled:pointer-events-none"
            title="Back"
          >
            <ArrowLeft size={14} />
          </button>
          <button
            onClick={navForward}
            disabled={!canGoForward}
            className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors disabled:opacity-25 disabled:pointer-events-none"
            title="Forward"
          >
            <ArrowRight size={14} />
          </button>
        </div>
      )}
      <Icon size={14} className="text-brand shrink-0" />
      <h1 className="text-xs font-mono uppercase tracking-widest text-content-primary leading-none">{title}</h1>
      {right && <div className="ml-auto flex items-center gap-3">{right}</div>}

      {/* Help + Connection indicator + Window Controls */}
      <div className={`flex items-center gap-2 pr-1 ${right ? '' : 'ml-auto'}`}>
        {/* Update available indicator (Discord-style green arrow) */}
        {updateVersion && (
          <button
            onClick={() => window.dispatchEvent(new CustomEvent('cloto-open-settings', { detail: { section: 'about' } }))}
            className="p-1 rounded bg-emerald-500/15 hover:bg-emerald-500/25 text-emerald-500 transition-colors"
            title={t('update_available_banner', {
              version: updateVersion,
              defaultValue: `ClotoCore ${updateVersion} is available`,
            })}
          >
            <ArrowUp size={14} />
          </button>
        )}
        {onHelp && (
          <button
            onClick={onHelp}
            className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-brand transition-colors"
            title="Help"
          >
            <HelpCircle size={14} />
          </button>
        )}
        {/* Connection status dot */}
        {!checking && (
          <div
            className="relative group flex items-center"
            title={connected ? 'Backend connected' : 'Backend unreachable'}
          >
            <StatusDot status={connected ? 'online' : 'error'} />
            {/* Tooltip */}
            <div className="absolute top-full right-0 mt-1 px-2 py-1 rounded bg-surface-primary border border-edge shadow-lg text-[9px] font-mono text-content-secondary whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none z-50">
              {connected ? 'Connected' : 'Backend unreachable'}
            </div>
          </div>
        )}

        {/* Window Controls (Tauri only) */}
        {isTauri && (
          <>
            <button
              onClick={minimizeWindow}
              className="p-1.5 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors"
            >
              <Minus size={14} />
            </button>
            <button
              onClick={toggleMaximizeWindow}
              className="p-1.5 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors"
            >
              <Square size={13} />
            </button>
            <button
              onClick={closeWindow}
              className="p-1.5 ml-1 rounded hover:bg-red-500/20 text-content-tertiary hover:text-red-500 transition-colors"
            >
              <X size={14} />
            </button>
          </>
        )}
      </div>
    </header>
  );
}
