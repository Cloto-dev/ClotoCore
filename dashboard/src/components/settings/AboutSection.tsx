import { useState } from 'react';
import { RefreshCw, Download, CheckCircle, AlertCircle } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { SectionCard } from './common';
import { isTauri, checkForUpdates, applyUpdate, UpdateInfo } from '../../lib/tauri';

type UpdateState = 'idle' | 'checking' | 'up-to-date' | 'available' | 'updating' | 'updated' | 'error';

export function AboutSection() {
  const [updateState, setUpdateState] = useState<UpdateState>('idle');
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [error, setError] = useState('');
  const [updateOutput, setUpdateOutput] = useState('');
  const { t } = useTranslation('settings');

  const handleCheck = async () => {
    setUpdateState('checking');
    setError('');
    try {
      const info = await checkForUpdates();
      setUpdateInfo(info);
      setUpdateState(info.available ? 'available' : 'up-to-date');
    } catch (err: any) {
      setError(err?.message || 'Failed to check for updates');
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
    } catch (err: any) {
      setError(err?.message || 'Failed to apply update');
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
          <p className="text-xs text-content-secondary leading-relaxed">
            {t('about.description')}
          </p>
          <div className="text-2xl font-mono font-black text-brand">v{__APP_VERSION__}</div>
        </div>
      </SectionCard>

      <SectionCard title={t('about.updates')}>
        <div className="space-y-3">
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
                className="text-[10px] text-content-tertiary hover:text-brand transition-colors"
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
                  <span className="text-content-tertiary font-normal">
                    ({formatDate(updateInfo.releaseDate)})
                  </span>
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
                <div className="text-[10px] text-content-tertiary font-mono bg-glass rounded-lg p-2 border border-edge">
                  {updateOutput.slice(0, 300)}
                </div>
              )}
              <p className="text-[10px] text-content-tertiary">{t('about.restart_hint')}</p>
            </div>
          )}

          {/* Error */}
          {updateState === 'error' && error && (
            <div className="flex items-center gap-2 text-xs text-red-400">
              <AlertCircle size={14} />
              {error}
            </div>
          )}
        </div>
      </SectionCard>

      <SectionCard title={t('about.license')}>
        <div className="space-y-2">
          <p className="text-xs text-content-secondary">{t('about.bsl')}</p>
          <p className="text-[10px] text-content-tertiary">{t('about.mit_convert')}</p>
        </div>
      </SectionCard>

      <SectionCard title={t('about.links')}>
        <div className="space-y-3">
          {[
            { labelKey: 'about.repository', value: 'github.com/Cloto-dev/ClotoCore', href: 'https://github.com/Cloto-dev/ClotoCore' },
            { labelKey: 'about.contact', value: 'ClotoCore@proton.me', href: 'mailto:ClotoCore@proton.me' },
          ].map(link => (
            <div key={link.labelKey} className="flex items-center justify-between">
              <span className="text-[10px] text-content-tertiary uppercase tracking-widest font-bold">{t(link.labelKey)}</span>
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
    </>
  );
}
