import { Check, Copy, Eye, EyeOff, RefreshCw } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ConfirmDialog } from '../../components/ui/ConfirmDialog';
import { SecretInput } from '../../components/ui/SecretInput';
import { useApiKey } from '../../contexts/ApiKeyContext';
import { useApi } from '../../hooks/useApi';
import { useAsyncAction } from '../../hooks/useAsyncAction';
import { api } from '../../services/api';
import { SettingsGroup, SettingsRow } from './common';
import { HubAccessToken } from './HubAccessToken';
import { LlmProvidersSection } from './LlmProvidersSection';
import { PanelWriteConsents } from './PanelWriteConsents';

export function SecuritySection() {
  const { setApiKey, forgetApiKey } = useApiKey();
  const authApi = useApi();
  const { t } = useTranslation('settings');
  const { t: tc } = useTranslation();
  const [newKey, setNewKey] = useState('');
  const [confirmInvalidate, setConfirmInvalidate] = useState(false);
  const [confirmRegenerate, setConfirmRegenerate] = useState(false);
  const [revealed, setRevealed] = useState(false);
  const [copied, setCopied] = useState(false);
  // Set when a rotation succeeded but will not survive a restart (the kernel
  // takes its key from the environment). Nothing else surfaces this: the call
  // returns 200 and the new key works until the kernel is restarted.
  const [rotationWarning, setRotationWarning] = useState<string | null>(null);

  const saveAction = useAsyncAction(t('security.error_invalid_key'));
  const invalidateAction = useAsyncAction(t('security.error_invalidate_failed'));
  const regenerateAction = useAsyncAction(t('security.error_regenerate_failed'));

  const error = saveAction.error || invalidateAction.error || regenerateAction.error;

  const handleCopy = async () => {
    if (!authApi.apiKey) return;
    try {
      await navigator.clipboard.writeText(authApi.apiKey);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard unavailable (non-secure context) — reveal instead.
      setRevealed(true);
    }
  };

  const handleRegenerate = () => {
    if (!authApi.apiKey) return;
    regenerateAction.run(async () => {
      const result = await authApi.regenerateApiKey();
      setApiKey(result.api_key);
      setConfirmRegenerate(false);
      setRevealed(true); // show the new key once so the user can save it
      setRotationWarning(result.survives_restart === false ? (result.warning ?? null) : null);
    });
  };

  const handleSave = () => {
    if (!newKey.trim()) return;
    saveAction.run(async () => {
      await api.listCronJobs(newKey.trim());
      setApiKey(newKey.trim());
      setNewKey('');
    });
  };

  const handleInvalidate = () => {
    if (!authApi.apiKey) return;
    invalidateAction.run(async () => {
      await authApi.invalidateApiKey();
      forgetApiKey();
      setConfirmInvalidate(false);
    });
  };

  const clearErrors = () => {
    saveAction.clearError();
    invalidateAction.clearError();
  };

  return (
    <>
      <SettingsGroup title={t('security.api_key_title')}>
        <SettingsRow label={t('security.status_label')}>
          <span className={authApi.apiKey ? 'st ok' : 'st warn'}>
            {authApi.apiKey ? t('security.configured') : t('security.not_configured')}
          </span>
        </SettingsRow>

        {/* Current key: reveal / copy / regenerate (admin-key handover,
            docs/ONBOARDING_MODERNIZATION_DESIGN.md §2.2) */}
        {authApi.apiKey && (
          <SettingsRow label={t('security.current_key')} desc={t('security.current_key_hint')}>
            <span className="keyline">
              <code className="select-all">{revealed ? authApi.apiKey : '••••••••••••••••'}</code>
              <button
                type="button"
                className="icb"
                onClick={() => setRevealed((v) => !v)}
                aria-label={t('security.reveal_key')}
              >
                {revealed ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
              <button type="button" className="icb" onClick={handleCopy} aria-label={t('security.copy_key')}>
                {copied ? <Check size={14} /> : <Copy size={14} />}
              </button>
              <button
                type="button"
                className="icb"
                onClick={() => setConfirmRegenerate(true)}
                disabled={regenerateAction.isLoading}
                aria-label={t('security.regenerate_label')}
              >
                <RefreshCw size={14} className={regenerateAction.isLoading ? 'animate-spin' : ''} />
              </button>
            </span>
          </SettingsRow>
        )}

        <SettingsRow label={t('security.new_key_label')} desc={t('security.new_key_desc')}>
          <SecretInput
            value={newKey}
            onChange={(v) => {
              setNewKey(v);
              clearErrors();
            }}
            placeholder={authApi.apiKey ? t('security.placeholder_replace') : t('security.placeholder_new')}
            className="in secret mono"
          />
          <button
            type="button"
            className="btn pri"
            onClick={handleSave}
            disabled={!newKey.trim() || saveAction.isLoading}
            aria-label={tc('save')}
          >
            {saveAction.isLoading ? '...' : tc('save')}
          </button>
        </SettingsRow>

        {error && <p className="says bad">{error}</p>}
        {rotationWarning && <p className="says warn">{rotationWarning}</p>}

        {authApi.apiKey && (
          <div className="set-block">
            <button
              type="button"
              className="btn danger"
              onClick={() => setConfirmInvalidate(true)}
              aria-label={t('security.invalidate_label')}
            >
              {t('security.invalidate_label')}
            </button>
            <p className="gdesc">{t('security.invalidate_desc')}</p>
          </div>
        )}
      </SettingsGroup>

      <ConfirmDialog
        open={confirmInvalidate}
        title={t('security.invalidate_label')}
        message={t('security.invalidate_confirm_desc')}
        confirmLabel={tc('confirm')}
        cancelLabel={tc('cancel')}
        variant="danger"
        onConfirm={handleInvalidate}
        onCancel={() => setConfirmInvalidate(false)}
      />

      <ConfirmDialog
        open={confirmRegenerate}
        title={t('security.regenerate_label')}
        message={t('security.regenerate_confirm_desc')}
        confirmLabel={tc('confirm')}
        cancelLabel={tc('cancel')}
        variant="danger"
        onConfirm={handleRegenerate}
        onCancel={() => setConfirmRegenerate(false)}
      />

      <HubAccessToken />

      <PanelWriteConsents />

      <LlmProvidersSection />
    </>
  );
}
