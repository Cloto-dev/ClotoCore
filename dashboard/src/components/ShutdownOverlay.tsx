import { Power } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { isTauri } from '../lib/tauri';
import { api } from '../services/api';

type Phase = 'hidden' | 'shutting-down' | 'stopped';

let externalTrigger: (() => void) | null = null;

/**
 * Show the full-screen shutdown overlay (Goal #164). Called by the sidebar
 * shutdown button; the tray-quit path triggers the same overlay through the
 * Tauri `shutdown-started` event instead. No-op if the overlay is not mounted.
 */
export function showShutdownOverlay() {
  externalTrigger?.();
}

/**
 * Run the safe shutdown from the dashboard UI (Goal #164): show the overlay,
 * then — desktop — invoke the shared `begin_shutdown` sequence (same path as
 * the tray Quit), or — browser — POST the kernel's shutdown endpoint.
 */
export async function requestShutdown(post: (path: string, payload: unknown) => Promise<void>) {
  showShutdownOverlay();
  try {
    if (isTauri) {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('shutdown_app');
    } else {
      await post('/system/shutdown', {});
    }
  } catch (err) {
    if (import.meta.env.DEV) console.error('Failed to shut down:', err);
  }
}

/**
 * Full-screen overlay shown while the kernel drains MCP servers and shuts
 * down. Mounted above the connection gate so it survives the backend going
 * away (in browser mode the boot screen would otherwise replace it).
 *
 * In the desktop app the process exits at the end of the sequence; in browser
 * mode the overlay polls health until the kernel stops answering, then shows
 * the terminal "stopped" screen.
 */
export function ShutdownOverlay() {
  const { t } = useTranslation('settings');
  const [phase, setPhase] = useState<Phase>('hidden');

  useEffect(() => {
    externalTrigger = () => setPhase((p) => (p === 'hidden' ? 'shutting-down' : p));
    return () => {
      externalTrigger = null;
    };
  }, []);

  // Tray-initiated shutdown (desktop): the Tauri backend emits shutdown-started
  // at the start of the shared sequence, whichever UI path began it.
  useEffect(() => {
    if (!isTauri) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    import('@tauri-apps/api/event')
      .then(({ listen }) => listen('shutdown-started', () => setPhase((p) => (p === 'hidden' ? 'shutting-down' : p))))
      .then(
        (fn) => {
          if (cancelled) fn();
          else unlisten = fn;
        },
        // Listener setup failure is non-fatal — the overlay still works for
        // button-initiated shutdowns via showShutdownOverlay().
        () => {},
      );
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Browser mode: this page outlives the kernel — poll health until the kernel
  // stops answering (or a hard cap elapses), then show the terminal screen.
  useEffect(() => {
    if (phase !== 'shutting-down' || isTauri) return;
    const started = Date.now();
    const timer = setInterval(() => {
      api.getHealth().then(
        () => {
          if (Date.now() - started > 30_000) setPhase('stopped');
        },
        () => setPhase('stopped'),
      );
    }, 1000);
    return () => clearInterval(timer);
  }, [phase]);

  if (phase === 'hidden') return null;

  const stopped = phase === 'stopped';

  return (
    <div className="fixed inset-0 z-[1000] bg-surface-base flex flex-col items-center justify-center select-none">
      <Power size={28} className={stopped ? 'text-content-tertiary' : 'text-brand animate-pulse'} />
      <h1 className="mt-4 text-2xl font-black tracking-[0.2em] text-content-primary uppercase">
        {stopped ? t('advanced.shutdown_overlay_stopped_title') : t('advanced.shutdown_overlay_title')}
      </h1>
      <p className="mt-3 text-[11px] font-mono text-brand/70 uppercase tracking-[0.15em]">
        {stopped ? t('advanced.shutdown_overlay_stopped_hint') : t('advanced.shutdown_overlay_draining')}
      </p>
    </div>
  );
}
