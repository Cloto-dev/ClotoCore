import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { extractError } from '../../lib/errors';
import type { PanelWriteConsent } from '../../types';
import { SettingsGroup, SettingsRow } from './common';

/**
 * Panels the operator has allowed to act for them, each with a way to take it
 * back (docs/PANEL_WRITE_GATE_DESIGN.md §4.6).
 *
 * Withdrawing takes effect on the next write the panel tries, because the kernel
 * reads the consent on every write — so it is done at once, without the
 * pending-then-save pattern the agent settings use.
 */
export function PanelWriteConsents() {
  const api = useApi();
  const { t } = useTranslation('settings');
  const [rows, setRows] = useState<PanelWriteConsent[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [revokeError, setRevokeError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setRows(await api.listModuleWriteConsents());
      setLoadError(null);
    } catch (e) {
      // Not an empty list: an empty list would say nothing is allowed.
      setRows(null);
      setLoadError(extractError(e, t('security.panel_consents_load_failed')));
    }
  }, [api, t]);

  useEffect(() => {
    void load();
  }, [load]);

  const revoke = async (panelId: string) => {
    setBusy(panelId);
    setRevokeError(null);
    try {
      await api.deleteModuleWriteConsent(panelId);
    } catch (e) {
      setRevokeError(extractError(e, t('security.panel_consents_revoke_failed')));
    } finally {
      setBusy(null);
      await load();
    }
  };

  return (
    <SettingsGroup title={t('security.panel_consents_title')}>
      <p className="gdesc">{t('security.panel_consents_desc')}</p>
      {loadError ? (
        <p className="says bad">{loadError}</p>
      ) : rows === null ? null : rows.length === 0 ? (
        <p className="gdesc">{t('security.panel_consents_empty')}</p>
      ) : (
        rows.map((row) => (
          <SettingsRow
            key={row.panel_id}
            label={<code>{row.panel_id}</code>}
            desc={t('security.panel_consents_granted', { date: new Date(row.granted_at).toLocaleString() })}
          >
            <button
              type="button"
              className="btn danger"
              onClick={() => void revoke(row.panel_id)}
              disabled={busy === row.panel_id}
            >
              {t('security.panel_consents_revoke')}
            </button>
          </SettingsRow>
        ))
      )}
      {revokeError && <p className="says bad">{revokeError}</p>}
    </SettingsGroup>
  );
}
