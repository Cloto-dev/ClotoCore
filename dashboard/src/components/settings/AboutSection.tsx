import { CheckCircle, Download, RefreshCw, RotateCcw } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { REPOSITORY_URL } from '../../constants';
import { useLocalStorage } from '../../hooks/useStorage';
import { applyUpdate, checkForUpdates, isTauri, type UpdateInfo } from '../../lib/tauri';
import { SetupWizard } from '../SetupWizard';
import { AlertCard } from '../ui/AlertCard';
import { SectionCard, Toggle } from './common';

type UpdateState = 'idle' | 'checking' | 'up-to-date' | 'available' | 'updating' | 'updated' | 'error';

export function AboutSection() {
  const [updateState, setUpdateState] = useState<UpdateState>('idle');
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [error, setError] = useState('');
  const [updateOutput, setUpdateOutput] = useState('');
  const [showWizard, setShowWizard] = useState(false);
  const { t } = useTranslation('settings');
  const [autoUpdateRaw, setAutoUpdateRaw] = useLocalStorage('cloto-auto-update', 'on');
  const autoUpdateEnabled = autoUpdateRaw !== 'off';

  const handleCheck = async () => {
    setUpdateState('checking');
    setError('');
    try {
      const info = await checkForUpdates();
      setUpdateInfo(info);
      setUpdateState(info.available ? 'available' : 'up-to-date');
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Failed to check for updates';
      setError(message);
      setUpdateState('error');
    }
  };

  const handleUpdate = async () => {
    setUpdateState('updating');
    setError('');
    try {
      const output = await applyUpdate();
      setUpdateOutput(output);
      setUpdateState('updated');
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Failed to apply update';
      setError(message);
      setUpdateState('error');
    }
  };

  const formatDate = (iso?: string) => {
    if (!iso) return '';
    try {
      return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
    } catch {
      return iso;
    }
  };

  return (
    <>
      <SectionCard title={t('about.clotocore')}>
        <div className="space-y-3">
          <p className="text-xs text-content-secondary leading-relaxed">{t('about.description')}</p>
          <div className="text-2xl font-mono font-black text-brand">v{__APP_VERSION__}</div>
        </div>
      </SectionCard>

      <SectionCard title={t('about.updates')}>
        <div className="space-y-3">
          {/* Auto-update toggle (Tauri desktop only) */}
          {isTauri && (
            <div className="mb-3 pb-3 border-b border-edge">
              <Toggle
                enabled={autoUpdateEnabled}
                onToggle={() => setAutoUpdateRaw(autoUpdateEnabled ? 'off' : 'on')}
                label={t('about.auto_update')}
              />
              <p className="text-[11px] text-content-tertiary mt-1">{t('about.auto_update_desc')}</p>
            </div>
          )}

          {/* Check button */}
          {(updateState === 'idle' || updateState === 'error') && (
            <button
              onClick={handleCheck}
              className="flex items-center gap-2 px-4 py-2 rounded-lg border border-edge text-xs font-bold text-content-secondary hover:text-brand hover:border-brand transition-all"
            >
              <RefreshCw size={14} />
              {t('about.check_for_updates')}
            </button>
          )}

          {/* Checking spinner */}
          {updateState === 'checking' && (
            <div className="flex items-center gap-2 text-xs text-content-tertiary">
              <RefreshCw size={14} className="animate-spin" />
              {t('about.checking')}
            </div>
          )}

          {/* Up to date */}
          {updateState === 'up-to-date' && (
            <div className="space-y-2">
              <div className="flex items-center gap-2 text-xs text-emerald-500 font-bold">
                <CheckCircle size={14} />
                {t('about.up_to_date', { version: updateInfo?.currentVersion })}
              </div>
              <button
                onClick={handleCheck}
                className="text-xs text-content-tertiary hover:text-brand transition-colors"
              >
                {t('about.check_again')}
              </button>
            </div>
          )}

          {/* Update available */}
          {updateState === 'available' && updateInfo && (
            <div className="space-y-3">
              <div className="flex items-center gap-2 text-xs text-brand font-bold">
                <Download size={14} />
                {t('about.available', { version: updateInfo.latestVersion })}
                {updateInfo.releaseDate && (
                  <span className="text-content-tertiary font-normal">({formatDate(updateInfo.releaseDate)})</span>
                )}
              </div>

              {updateInfo.releaseNotes && (
                <div className="text-[11px] text-content-tertiary font-mono bg-glass rounded-lg p-3 border border-edge leading-relaxed max-h-32 overflow-y-auto">
                  {updateInfo.releaseNotes.slice(0, 500)}
                  {updateInfo.releaseNotes.length > 500 && '...'}
                </div>
              )}

              <div className="flex gap-2">
                {isTauri && (
                  <button
                    onClick={handleUpdate}
                    className="flex items-center gap-2 px-4 py-2 rounded-lg bg-brand text-white text-xs font-bold shadow-sm hover:shadow-md transition-all"
                  >
                    <Download size={14} />
                    {t('about.update_now')}
                  </button>
                )}
                <button
                  onClick={handleCheck}
                  className="px-4 py-2 rounded-lg border border-edge text-xs font-bold text-content-secondary hover:text-brand transition-all"
                >
                  {t('about.recheck')}
                </button>
              </div>
            </div>
          )}

          {/* Updating */}
          {updateState === 'updating' && (
            <div className="flex items-center gap-2 text-xs text-content-tertiary">
              <RefreshCw size={14} className="animate-spin" />
              {t('about.applying')}
            </div>
          )}

          {/* Updated */}
          {updateState === 'updated' && (
            <div className="space-y-2">
              <div className="flex items-center gap-2 text-xs text-emerald-500 font-bold">
                <CheckCircle size={14} />
                {t('about.applied')}
              </div>
              {updateOutput && (
                <div className="text-xs text-content-tertiary font-mono bg-glass rounded-lg p-2 border border-edge">
                  {updateOutput.slice(0, 300)}
                </div>
              )}
              <p className="text-xs text-content-tertiary">{t('about.restart_hint')}</p>
            </div>
          )}

          {/* Error */}
          {updateState === 'error' && error && <AlertCard>{error}</AlertCard>}
        </div>
      </SectionCard>

      <SectionCard title={t('about.license')}>
        <div className="space-y-2">
          <p className="text-xs text-content-secondary">{t('about.bsl')}</p>
          <p className="text-xs text-content-tertiary">{t('about.mit_convert')}</p>
        </div>
      </SectionCard>

      <SectionCard title={t('about.links')}>
        <div className="space-y-3">
          {[
            {
              labelKey: 'about.repository',
              value: 'github.com/Cloto-dev/ClotoCore',
              href: REPOSITORY_URL,
            },
            { labelKey: 'about.contact', value: 'ClotoCore@proton.me', href: 'mailto:ClotoCore@proton.me' },
          ].map((link) => (
            <div key={link.labelKey} className="flex items-center justify-between">
              <span className="text-xs text-content-tertiary uppercase tracking-widest font-bold">
                {t(link.labelKey)}
              </span>
              <a
                href={link.href}
                target="_blank"
                rel="noopener noreferrer"
                className="text-xs text-brand hover:underline font-mono"
              >
                {link.value}
              </a>
            </div>
          ))}
        </div>
      </SectionCard>

      <SectionCard title={t('about.setup')}>
        <div className="space-y-2">
          <p className="text-xs text-content-tertiary">{t('about.setup_desc')}</p>
          <button
            onClick={() => setShowWizard(true)}
            className="flex items-center gap-2 px-4 py-2 rounded-lg border border-edge text-xs font-bold text-content-secondary hover:text-brand hover:border-brand transition-all"
          >
            <RotateCcw size={14} />
            {t('about.rerun_setup')}
          </button>
        </div>
      </SectionCard>

      {showWizard && (
        <SetupWizard
          onComplete={() => {
            setShowWizard(false);
            window.dispatchEvent(new CustomEvent('cloto-setup-rerun-complete'));
          }}
        />
      )}
    </>
  );
}
