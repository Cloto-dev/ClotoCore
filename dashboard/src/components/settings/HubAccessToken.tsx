import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation } from 'react-router-dom';
import { ConfirmDialog } from '../../components/ui/ConfirmDialog';
import { SecretInput } from '../../components/ui/SecretInput';
import { useApi } from '../../hooks/useApi';
import { extractError } from '../../lib/errors';
import type { HubAccessStatus } from '../../services/api';
import { SettingsGroup, SettingsRow } from './common';

/**
 * Enough of the key fingerprint to tell kernels apart at a glance: the first
 * 16 hex characters in groups of four. The full value is in the tooltip.
 */
export function shortFingerprint(fingerprint: string): string {
  return `${(fingerprint.slice(0, 16).match(/.{1,4}/g) ?? []).join(' ')}…`;
}

/** The anchor a hub access notice links to (the kernel's notice metadata). */
export const HUB_ACCESS_ANCHOR = 'hub-access';

/**
 * This kernel's hub access token: set, status, renew, forget
 * (docs/HUB_ACCESS_DESIGN.md §4).
 *
 * Acts at once rather than through a pending-then-save step: like the API key
 * above it, each action is a single call whose result is shown immediately,
 * and forgetting is behind a confirmation.
 *
 * The pasted token is cleared from the field as soon as it has been sent,
 * whether or not the hub accepted it — a secret left sitting in an input is a
 * secret on screen. An expired token offers no renewal: the hub refuses to
 * renew one, so the page says to ask for a new token instead.
 */
export function HubAccessToken() {
  const api = useApi();
  const { t } = useTranslation('settings');
  const { t: tc } = useTranslation();
  const location = useLocation();
  const groupRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<HubAccessStatus | null | undefined>(undefined);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [draft, setDraft] = useState('');
  const [busy, setBusy] = useState(false);
  const [confirmForget, setConfirmForget] = useState(false);

  const load = useCallback(async () => {
    try {
      setStatus(await api.getHubAccess());
      setLoadError(null);
    } catch (e) {
      // Not "no token": that would tell the operator to paste one they have.
      setStatus(undefined);
      setLoadError(extractError(e, t('security.hub_access_load_failed')));
    }
  }, [api, t]);

  useEffect(() => {
    void load();
  }, [load]);

  // Arriving from a notice: bring this group into view.
  useEffect(() => {
    if (location.hash === `#${HUB_ACCESS_ANCHOR}`) groupRef.current?.scrollIntoView?.({ block: 'start' });
  }, [location.hash]);

  const run = async (action: () => Promise<void>, fallback: string) => {
    setBusy(true);
    setActionError(null);
    try {
      await action();
    } catch (e) {
      setActionError(extractError(e, fallback));
    } finally {
      setBusy(false);
    }
  };

  const setToken = () => {
    const token = draft.trim();
    if (!token) return;
    setDraft('');
    void run(async () => {
      setStatus(await api.setHubAccessToken(token));
    }, t('security.hub_access_set_failed'));
  };

  const renew = () =>
    void run(async () => {
      setStatus(await api.renewHubAccessToken());
    }, t('security.hub_access_renew_failed'));

  const forget = () => {
    setConfirmForget(false);
    void run(async () => {
      await api.forgetHubAccessToken();
      setStatus(null);
    }, t('security.hub_access_forget_failed'));
  };

  const date = status ? new Date(status.expires_at).toLocaleDateString() : '';

  return (
    <div ref={groupRef} id={HUB_ACCESS_ANCHOR}>
      <SettingsGroup title={t('security.hub_access_title')}>
        <p className="gdesc">{t('security.hub_access_desc')}</p>

        {loadError && <p className="says bad">{loadError}</p>}
        {status === null && <p className="gdesc">{t('security.hub_access_none')}</p>}

        {status && (
          <>
            <SettingsRow label={t('security.hub_access_connectors')}>
              <span data-testid="hub-access-connectors">
                {status.connector_ids.map((id, i) => (
                  <span key={id}>
                    {i > 0 && ', '}
                    <code>{id}</code>
                  </span>
                ))}
              </span>
            </SettingsRow>

            <SettingsRow label={t('security.hub_access_expires')} desc={t(`security.hub_access_${status.stage}_desc`)}>
              <span
                data-testid="hub-access-expiry"
                className={
                  status.stage === 'expired' ? 'st bad' : status.stage === 'expires_soon' ? 'st warn' : 'st ok'
                }
              >
                {t(`security.hub_access_${status.stage}`, { date })}
              </span>
              {/* No renewal once expired: the hub refuses it (design §2). */}
              {status.stage !== 'expired' && (
                <button type="button" className="btn" onClick={renew} disabled={busy}>
                  {t('security.hub_access_renew')}
                </button>
              )}
            </SettingsRow>

            <SettingsRow label={t('security.hub_access_fingerprint')} desc={t('security.hub_access_fingerprint_desc')}>
              <code title={status.fingerprint} data-testid="hub-access-fingerprint">
                {shortFingerprint(status.fingerprint)}
              </code>
            </SettingsRow>
          </>
        )}

        {status !== undefined && (
          <SettingsRow
            label={status ? t('security.hub_access_replace_label') : t('security.hub_access_set_label')}
            desc={t('security.hub_access_set_desc')}
          >
            <SecretInput
              value={draft}
              onChange={(v) => {
                setDraft(v);
                setActionError(null);
              }}
              placeholder="chubr_…"
              className="in secret mono"
            />
            <button type="button" className="btn pri" onClick={setToken} disabled={!draft.trim() || busy}>
              {t('security.hub_access_set')}
            </button>
          </SettingsRow>
        )}

        {actionError && <p className="says bad">{actionError}</p>}

        {status && (
          <div className="set-block">
            <button type="button" className="btn danger" onClick={() => setConfirmForget(true)} disabled={busy}>
              {t('security.hub_access_forget')}
            </button>
          </div>
        )}
      </SettingsGroup>

      <ConfirmDialog
        open={confirmForget}
        title={t('security.hub_access_forget')}
        message={t('security.hub_access_forget_confirm')}
        confirmLabel={tc('confirm')}
        cancelLabel={tc('cancel')}
        variant="danger"
        onConfirm={forget}
        onCancel={() => setConfirmForget(false)}
      />
    </div>
  );
}
