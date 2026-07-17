import { Check, Copy, Eye, EyeOff, RefreshCw } from 'lucide-react';
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
  const [confirmRegenerate, setConfirmRegenerate] = useState(false);
  const [revealed, setRevealed] = useState(false);
  const [copied, setCopied] = useState(false);

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
      <SectionCard title={t('security.api_key_title')}>
        <div className="space-y-4">
          <div className="flex items-center gap-2">
            <div
              role="img"
              className={`w-2 h-2 rounded-full ${authApi.apiKey ? 'bg-green-500' : 'bg-amber-500'}`}
              aria-label={authApi.apiKey ? t('security.configured') : t('security.not_configured')}
            />
            <span className="text-xs text-content-secondary">
              {authApi.apiKey ? t('security.configured') : t('security.not_configured')}
            </span>
          </div>

          {/* Current key: reveal / copy / regenerate (admin-key handover,
              docs/ONBOARDING_MODERNIZATION_DESIGN.md §2.2) */}
          {authApi.apiKey && (
            <div className="space-y-1.5">
              <span className="text-[10px] font-bold text-content-tertiary uppercase tracking-wider">
                {t('security.current_key')}
              </span>
              <div className="flex items-center gap-2">
                <code className="flex-1 truncate bg-surface-secondary border border-edge rounded-lg px-3 py-2 text-xs font-mono text-content-primary select-all">
                  {revealed ? authApi.apiKey : '••••••••••••••••'}
                </code>
                <button
                  type="button"
                  onClick={() => setRevealed((v) => !v)}
                  aria-label={t('security.reveal_key')}
                  className="p-2 rounded-lg bg-surface-secondary border border-edge text-content-secondary hover:text-content-primary transition-colors"
                >
                  {revealed ? <EyeOff size={14} /> : <Eye size={14} />}
                </button>
                <button
                  type="button"
                  onClick={handleCopy}
                  aria-label={t('security.copy_key')}
                  className="p-2 rounded-lg bg-surface-secondary border border-edge text-content-secondary hover:text-content-primary transition-colors"
                >
                  {copied ? <Check size={14} className="text-green-500" /> : <Copy size={14} />}
                </button>
                <button
                  type="button"
                  onClick={() => setConfirmRegenerate(true)}
                  disabled={regenerateAction.isLoading}
                  aria-label={t('security.regenerate_label')}
                  className="p-2 rounded-lg bg-surface-secondary border border-edge text-amber-500 hover:text-amber-400 transition-colors disabled:opacity-50"
                >
                  <RefreshCw size={14} className={regenerateAction.isLoading ? 'animate-spin' : ''} />
                </button>
              </div>
              <p className="text-[11px] text-content-tertiary">{t('security.current_key_hint')}</p>
            </div>
          )}

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
              aria-label={tc('save')}
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
                aria-label={t('security.invalidate_label')}
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

      <LlmProvidersSection />
    </>
  );
}
