import { AlertTriangle, RefreshCw } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';
import { AlertCard } from '../components/ui/AlertCard';
import { useApi } from '../hooks/useApi';
import { useModules } from '../hooks/useModules';
import { extractError } from '../lib/errors';
import { decideModuleCall, MODULE_RESULT } from '../lib/moduleBridge';

/**
 * Host for one runtime-loaded UI module.
 *
 * The module's document is fetched here, with the operator's credential, and
 * handed to a sandboxed frame as text. The frame is given `allow-scripts` and
 * nothing else, so it runs in an opaque origin: no cookie, no admin key, no
 * storage, and no way to call the kernel behind the operator's back. When it
 * needs data it posts a message; `lib/moduleBridge` decides whether that call
 * is one the module's manifest declared, and only then does the host make it.
 *
 * The cost of that isolation is that a module is one self-contained document —
 * a relative `<script src>` has no origin to resolve against. That is the trade
 * that was chosen: a module can be added without rebuilding the kernel, and it
 * cannot quietly borrow the operator's authority.
 */
export function ModulePage() {
  const { id = '' } = useParams();
  const api = useApi();
  const { t } = useTranslation('nav');
  const { modules, isLoading: listLoading } = useModules();
  const frameRef = useRef<HTMLIFrameElement>(null);

  const [document, setDocument] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  const entry = modules.find((m) => m.id === id);
  // Read through a ref so the message listener does not have to be torn down
  // and rebuilt every time the module list refreshes.
  const requiresRef = useRef<string[]>([]);
  requiresRef.current = entry?.requires ?? [];

  const load = useCallback(async () => {
    if (!id) return;
    setIsLoading(true);
    setLoadError(null);
    try {
      setDocument(await api.fetchModuleDocument(id, entry?.entry || 'index.html'));
    } catch (e) {
      setDocument(null);
      setLoadError(extractError(e, 'Failed to load module'));
    } finally {
      setIsLoading(false);
    }
  }, [api, id, entry?.entry]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      const frame = frameRef.current;
      // The frame has an opaque origin, so `event.origin` is the string "null"
      // for every sandboxed frame on the page and cannot identify this one.
      // The window reference can, and is the check that matters.
      if (!frame || event.source !== frame.contentWindow) return;

      const decision = decideModuleCall(event.data, requiresRef.current);
      // `targetOrigin: "*"` because an opaque origin cannot be named. It is safe
      // here only because the recipient is addressed by window reference, not
      // broadcast: `contentWindow` is this frame and no other.
      const reply = (payload: Record<string, unknown>) =>
        frame.contentWindow?.postMessage({ cloto: MODULE_RESULT, ...payload }, '*');

      if (!decision.allowed) {
        if (decision.id) reply({ id: decision.id, ok: false, error: decision.reason });
        return;
      }
      const { id: requestId, method, path } = decision.request;
      api
        .callForModule(method, path)
        .then(({ status, body }) => reply({ id: requestId, ok: status < 400, status, body }))
        .catch((e) => reply({ id: requestId, ok: false, error: extractError(e, 'Request failed') }));
    };
    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  }, [api]);

  const title = entry?.name || id;
  const rejected = entry?.error;

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-3 px-4 py-3 border-b border-edge bg-glass">
        <div className="min-w-0">
          <h1 className="text-sm font-mono font-bold text-content-primary truncate">{title}</h1>
          {entry?.description && <p className="text-[10px] text-content-tertiary truncate">{entry.description}</p>}
        </div>
        <button
          type="button"
          onClick={() => void load()}
          disabled={isLoading}
          aria-label={t('module_reload')}
          title={t('module_reload')}
          className="ml-auto p-2 rounded-lg border border-edge bg-glass text-content-secondary hover:text-brand hover:border-brand disabled:opacity-30"
        >
          <RefreshCw size={14} className={isLoading ? 'animate-spin' : ''} />
        </button>
      </div>

      <div className="flex-1 min-h-0 p-4">
        {rejected ? (
          <AlertCard variant="error">
            <span className="flex items-start gap-2">
              <AlertTriangle size={12} className="flex-shrink-0 mt-0.5" />
              <span>
                {t('module_rejected')}: {rejected}
              </span>
            </span>
          </AlertCard>
        ) : loadError ? (
          <AlertCard variant="error">
            <span className="flex items-start gap-2">
              <AlertTriangle size={12} className="flex-shrink-0 mt-0.5" />
              <span>
                {t('module_load_failed')}: {loadError}
              </span>
            </span>
          </AlertCard>
        ) : !entry && !listLoading ? (
          <AlertCard variant="error">
            <span className="flex items-start gap-2">
              <AlertTriangle size={12} className="flex-shrink-0 mt-0.5" />
              <span>
                {t('module_missing')}: {id}
              </span>
            </span>
          </AlertCard>
        ) : document !== null ? (
          <iframe
            ref={frameRef}
            title={title}
            srcDoc={document}
            sandbox="allow-scripts"
            className="w-full h-full rounded-lg border border-edge bg-glass"
          />
        ) : null}
      </div>
    </div>
  );
}
