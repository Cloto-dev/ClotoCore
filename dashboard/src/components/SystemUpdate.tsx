import { useState, useEffect, useRef } from 'react';
import { RefreshCw, Download, CheckCircle, AlertCircle, Loader2 } from 'lucide-react';
import { api, UpdateInfo } from '../services/api';

const isTauri = '__TAURI_INTERNALS__' in window;

type UpdateState = 'idle' | 'checking' | 'available' | 'up-to-date' | 'applying' | 'done' | 'error';

// M-24: Timeout for update operations
const UPDATE_TIMEOUT_MS = 30000;

export function SystemUpdate() {
  const [state, setState] = useState<UpdateState>('idle');
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [currentVersion, setCurrentVersion] = useState<string>('');
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    api.getVersion()
      .then(v => setCurrentVersion(v.version))
      .catch(() => {});
    return () => { abortRef.current?.abort(); };
  }, []);

  const checkForUpdateTauri = async () => {
    setState('checking');
    setError(null);
    try {
      const { check } = await import('@tauri-apps/plugin-updater');
      const update = await check();
      if (update) {
        setInfo({
          current_version: update.currentVersion,
          latest_version: update.version,
          update_available: true,
          release_notes: update.body ?? undefined,
        });
        setState('available');
      } else {
        setInfo({ current_version: currentVersion, update_available: false });
        setState('up-to-date');
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to check for updates');
      setState('error');
    }
  };

  const applyUpdateTauri = async () => {
    setState('applying');
    setError(null);
    try {
      const { check } = await import('@tauri-apps/plugin-updater');
      const update = await check();
      if (update) {
        await update.downloadAndInstall();
        setState('done');
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to apply update');
      setState('error');
    }
  };

  const checkForUpdateHttp = async () => {
    abortRef.current?.abort();
    abortRef.current = new AbortController();
    const timeoutId = setTimeout(() => abortRef.current?.abort(), UPDATE_TIMEOUT_MS);

    setState('checking');
    setError(null);
    try {
      const result = await api.checkForUpdate();
      if (!abortRef.current.signal.aborted) {
        setInfo(result);
        setState(result.update_available ? 'available' : 'up-to-date');
      }
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') {
        setError('Update check timed out');
      } else {
        setError(e instanceof Error ? e.message : 'Failed to check for updates');
      }
      setState('error');
    } finally {
      clearTimeout(timeoutId);
    }
  };

  const applyUpdateHttp = async () => {
    if (!info?.latest_version) return;
    abortRef.current?.abort();
    abortRef.current = new AbortController();
    const timeoutId = setTimeout(() => abortRef.current?.abort(), UPDATE_TIMEOUT_MS * 2);

    setState('applying');
    setError(null);
    try {
      await api.applyUpdate(info.latest_version);
      if (!abortRef.current.signal.aborted) {
        setState('done');
      }
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') {
        setError('Update apply timed out');
      } else {
        setError(e instanceof Error ? e.message : 'Failed to apply update');
      }
      setState('error');
    } finally {
      clearTimeout(timeoutId);
    }
  };

  const checkForUpdate = isTauri ? checkForUpdateTauri : checkForUpdateHttp;
  const applyUpdate = isTauri ? applyUpdateTauri : applyUpdateHttp;

  return (
    <div className="p-4 space-y-4">
      {/* Version display */}
      <div className="flex items-center justify-between">
        <div>
          <div className="text-[10px] font-mono text-blue-400/60 tracking-widest uppercase">System Version</div>
          <div className="text-sm font-bold text-blue-300 font-mono">
            v{currentVersion || '...'}
          </div>
        </div>
        <button
          onClick={checkForUpdate}
          disabled={state === 'checking' || state === 'applying'}
          className="flex items-center gap-2 px-3 py-1.5 text-[10px] font-bold tracking-wider uppercase rounded border border-blue-500/30 text-blue-400 hover:bg-blue-500/10 hover:border-blue-400/50 transition-all disabled:opacity-30 disabled:cursor-not-allowed"
        >
          {state === 'checking' ? (
            <Loader2 size={12} className="animate-spin" />
          ) : (
            <RefreshCw size={12} />
          )}
          Check for Updates
        </button>
      </div>

      {/* Status */}
      {state === 'up-to-date' && (
        <div className="flex items-center gap-2 p-3 rounded bg-green-500/10 border border-green-500/20">
          <CheckCircle size={14} className="text-green-400" />
          <span className="text-[11px] text-green-400 font-mono">Up to date (v{info?.current_version})</span>
        </div>
      )}

      {state === 'available' && info && (
        <div className="space-y-3 p-3 rounded bg-blue-500/10 border border-blue-500/20">
          <div className="flex items-center justify-between">
            <div>
              <div className="text-[11px] font-bold text-blue-300 font-mono">
                v{info.latest_version} available
              </div>
              {info.release_name && (
                <div className="text-[10px] text-blue-400/60 mt-0.5">{info.release_name}</div>
              )}
              {info.published_at && (
                <div className="text-[10px] text-blue-400/40 mt-0.5">
                  {new Date(info.published_at).toLocaleDateString()}
                </div>
              )}
            </div>
            <button
              onClick={applyUpdate}
              className="flex items-center gap-2 px-3 py-1.5 text-[10px] font-bold tracking-wider uppercase rounded bg-blue-600 text-white hover:bg-blue-500 transition-all"
            >
              <Download size={12} />
              Apply Update
            </button>
          </div>
          {info.release_notes && (
            <div className="text-[10px] text-blue-400/70 font-mono whitespace-pre-wrap max-h-24 overflow-y-auto border-t border-blue-500/10 pt-2 no-scrollbar">
              {info.release_notes.slice(0, 500)}
            </div>
          )}
        </div>
      )}

      {state === 'applying' && (
        <div className="flex items-center gap-2 p-3 rounded bg-yellow-500/10 border border-yellow-500/20">
          <Loader2 size={14} className="animate-spin text-yellow-400" />
          <span className="text-[11px] text-yellow-400 font-mono">Downloading and applying update...</span>
        </div>
      )}

      {state === 'done' && (
        <div className="space-y-2 p-3 rounded bg-green-500/10 border border-green-500/20">
          <div className="flex items-center gap-2">
            <CheckCircle size={14} className="text-green-400" />
            <span className="text-[11px] text-green-400 font-mono">Update applied. System is restarting...</span>
          </div>
          <div className="text-[10px] text-green-400/60 font-mono">
            Refresh the page after restart completes.
          </div>
        </div>
      )}

      {state === 'error' && error && (
        <div className="flex items-center gap-2 p-3 rounded bg-red-500/10 border border-red-500/20">
          <AlertCircle size={14} className="text-red-400" />
          <span className="text-[11px] text-red-400 font-mono">{error}</span>
        </div>
      )}
    </div>
  );
}
