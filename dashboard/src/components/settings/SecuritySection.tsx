import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AlertCard } from '../../components/ui/AlertCard';
import { ConfirmDialog } from '../../components/ui/ConfirmDialog';
import { SecretInput } from '../../components/ui/SecretInput';
import { useApiKey } from '../../contexts/ApiKeyContext';
import { useApi } from '../../hooks/useApi';
import { useAsyncAction } from '../../hooks/useAsyncAction';
import { api } from '../../services/api';
import { SectionCard } from './common';
import { LlmProvidersSection } from './LlmProvidersSection';

export function SecuritySection() {
  const { setApiKey, forgetApiKey } = useApiKey();
  const authApi = useApi();
  const { t } = useTranslation('settings');
  const { t: tc } = useTranslation();
  const [newKey, setNewKey] = useState('');
  const [confirmInvalidate, setConfirmInvalidate] = useState(false);

  const saveAction = useAsyncAction(t('security.error_invalid_key'));
  const invalidateAction = useAsyncAction(t('security.error_invalidate_failed'));

  const error = saveAction.error || invalidateAction.error;

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
      <SectionCard title={t('security.api_key_title')}>
        <div className="space-y-4">
          <div className="flex items-center gap-2">
            <div className={`w-2 h-2 rounded-full ${authApi.apiKey ? 'bg-green-500' : 'bg-amber-500'}`} />
            <span className="text-xs text-content-secondary">
              {authApi.apiKey ? t('security.configured') : t('security.not_configured')}
            </span>
          </div>

          <div className="flex gap-2">
            <SecretInput
              value={newKey}
              onChange={(v) => {
                setNewKey(v);
                clearErrors();
              }}
              placeholder={authApi.apiKey ? t('security.placeholder_replace') : t('security.placeholder_new')}
              className="w-full bg-surface-secondary border border-edge rounded-lg px-3 py-2 pr-8 text-xs font-mono text-content-primary placeholder:text-content-tertiary focus:outline-none focus:border-brand transition-colors"
            />
            <button
              onClick={handleSave}
              disabled={!newKey.trim() || saveAction.isLoading}
              className="px-4 py-2 bg-brand text-white text-xs font-bold rounded-lg disabled:opacity-40 hover:bg-brand/90 transition-colors"
            >
              {saveAction.isLoading ? '...' : tc('save')}
            </button>
          </div>

          {error && <AlertCard>{error}</AlertCard>}

          {authApi.apiKey && (
            <div className="pt-3 border-t border-edge">
              <button
                onClick={() => setConfirmInvalidate(true)}
                className="text-xs text-red-400 hover:text-red-300 font-bold uppercase tracking-widest transition-colors"
              >
                {t('security.invalidate_label')}
              </button>
            </div>
          )}
        </div>
      </SectionCard>

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

      <LlmProvidersSection />
    </>
  );
}
