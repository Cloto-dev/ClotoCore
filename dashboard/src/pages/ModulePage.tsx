import { AlertTriangle, RefreshCw } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';
import { AlertCard } from '../components/ui/AlertCard';
import { useApi } from '../hooks/useApi';
import { useModules } from '../hooks/useModules';
import { extractError } from '../lib/errors';
import { decideModuleCall, MODULE_RESULT } from '../lib/moduleBridge';
import { describeWrite } from '../lib/panelWrites';
import type { ModuleWriteAccess } from '../types';

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
 * A module that declares `writes` may also change kernel state, but only
 * through the kernel's write relay, which re-checks the panel and the
 * operator's consent on every write (docs/PANEL_WRITE_GATE_DESIGN.md). This page
 * asks for that consent and shows that it was given; it never sends a write
 * anywhere else.
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
  const writesRef = useRef<string[]>([]);
  writesRef.current = entry?.writes ?? [];
  const idRef = useRef(id);
  idRef.current = id;

  const declaresWrites = (entry?.writes?.length ?? 0) > 0;
  const [access, setAccess] = useState<ModuleWriteAccess | null>(null);
  const [consentError, setConsentError] = useState<string | null>(null);
  const [consentDismissed, setConsentDismissed] = useState(false);
  const [showWrites, setShowWrites] = useState(false);

  const loadAccess = useCallback(async () => {
    if (!id || !declaresWrites) {
      setAccess(null);
      return;
    }
    try {
      setAccess(await api.getModuleWriteAccess(id));
    } catch {
      // Unknown is not "cannot write" and not "can": show neither the sheet nor
      // the badge. The kernel still refuses every write it has not admitted.
      setAccess(null);
    }
  }, [api, id, declaresWrites]);

  useEffect(() => {
    void loadAccess();
  }, [loadAccess]);

  const allowWrites = async () => {
    setConsentError(null);
    try {
      await api.putModuleWriteConsent(id);
      await loadAccess();
    } catch (e) {
      setConsentError(extractError(e, t('module_write_consent_failed')));
    }
  };

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

      const decision = decideModuleCall(event.data, requiresRef.current, writesRef.current);
      // `targetOrigin: "*"` because an opaque origin cannot be named. It is safe
      // here only because the recipient is addressed by window reference, not
      // broadcast: `contentWindow` is this frame and no other.
      const reply = (payload: Record<string, unknown>) =>
        frame.contentWindow?.postMessage({ cloto: MODULE_RESULT, ...payload }, '*');

      if (!decision.allowed) {
        if (decision.id) reply({ id: decision.id, ok: false, error: decision.reason });
        return;
      }
      const { id: requestId, method, path, write, body } = decision.request;
      // A write goes to the kernel's relay and nowhere else; the kernel decides
      // whether it happens.
      const call = write ? api.writeForModule(idRef.current, method, path, body) : api.callForModule(method, path);
      call
        .then(({ status, body }) => reply({ id: requestId, ok: status < 400, status, body }))
        .catch((e) => reply({ id: requestId, ok: false, error: extractError(e, 'Request failed') }));
    };
    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  }, [api]);

  const title = entry?.name || id;
  const rejected = entry?.error;
  const canWrite = access?.eligible === true && access.consent?.valid === true;
  const asksConsent = access?.eligible === true && access.consent?.valid !== true && !consentDismissed;

  const writeLines = (access?.writes ?? []).map((w) => {
    const d = describeWrite(w);
    if (d.kind === 'send_messages') return { key: w, text: t('module_write_send_messages', { agent: d.agent }) };
    if (d.kind === 'start_conversations')
      return { key: w, text: t('module_write_start_conversations', { agent: d.agent }) };
    return { key: w, text: d.entry };
  });

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-3 px-4 py-3 border-b border-edge bg-surface-panel">
        <div className="min-w-0">
          <h1 className="text-sm font-mono font-bold text-content-primary truncate">{title}</h1>
          {entry?.description && <p className="text-xs text-content-tertiary truncate">{entry.description}</p>}
        </div>
        {canWrite && (
          <button
            type="button"
            onClick={() => setShowWrites((v) => !v)}
            aria-expanded={showWrites}
            className="ml-auto px-2 py-1 rounded-md border border-edge text-xs text-content-secondary hover:border-edge"
          >
            {t('module_can_write')}
          </button>
        )}
        <button
          type="button"
          onClick={() => void load()}
          disabled={isLoading}
          aria-label={t('module_reload')}
          title={t('module_reload')}
          className={`${canWrite ? '' : 'ml-auto '}p-2 rounded-lg border border-edge bg-surface-panel text-content-secondary hover:text-agent hover:border-agent disabled:opacity-30`}
        >
          <RefreshCw size={14} className={isLoading ? 'animate-spin' : ''} />
        </button>
      </div>

      {canWrite && showWrites && (
        <div className="px-4 py-3 border-b border-edge bg-surface-panel text-xs text-content-secondary">
          <p>{t('module_write_list_title')}</p>
          <ul className="mt-1 list-disc pl-5">
            {writeLines.map((l) => (
              <li key={l.key}>{l.text}</li>
            ))}
          </ul>
        </div>
      )}

      {access && !access.eligible && (
        // A sentence, not an identifier, so not the monospace AlertCard.
        <p className="px-4 pt-4 text-xs text-amber-400">
          {t('module_write_ineligible', { reason: access.reason ?? '' })}
        </p>
      )}

      {asksConsent && (
        <section
          aria-label={t('module_write_consent_title')}
          className="mx-4 mt-4 p-3 rounded-lg border border-edge bg-surface-panel text-xs text-content-secondary"
        >
          <h2 className="text-sm font-bold text-content-primary">{t('module_write_consent_title')}</h2>
          {access?.consent && !access.consent.valid && <p className="mt-1">{t('module_write_consent_lapsed')}</p>}
          <p className="mt-1">{t('module_write_consent_desc')}</p>
          <ul className="mt-2 list-disc pl-5">
            {writeLines.map((l) => (
              <li key={l.key}>{l.text}</li>
            ))}
          </ul>
          {consentError && <p className="mt-2 text-red-400">{consentError}</p>}
          <div className="mt-3 flex gap-2">
            <button
              type="button"
              onClick={() => void allowWrites()}
              className="px-3 py-1 rounded-md border border-edge bg-surface-primary text-content-primary hover:border-edge"
            >
              {t('module_write_allow')}
            </button>
            <button
              type="button"
              onClick={() => setConsentDismissed(true)}
              className="px-3 py-1 rounded-md border border-edge hover:border-edge"
            >
              {t('module_write_not_now')}
            </button>
          </div>
        </section>
      )}

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
            className="w-full h-full rounded-lg border border-edge bg-surface-panel"
          />
        ) : null}
      </div>
    </div>
  );
}
