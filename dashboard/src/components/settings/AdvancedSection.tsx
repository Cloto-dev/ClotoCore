import { AlertTriangle, Power } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { isTauri } from '../../lib/tauri';
import { showShutdownOverlay } from '../ShutdownOverlay';
import { ConfirmDialog } from '../ui/ConfirmDialog';
import { SectionCard, Toggle } from './common';

export function AdvancedSection() {
  const api = useApi();
  const { t } = useTranslation('settings');
  const [yoloEnabled, setYoloEnabled] = useState(false);
  const [loading, setLoading] = useState(true);
  const [maxCronGen, setMaxCronGen] = useState(2);
  const [shutdownConfirm, setShutdownConfirm] = useState(false);

  useEffect(() => {
    api
      .fetchJson<{ enabled: boolean }>('/settings/yolo')
      .then((data) => setYoloEnabled(data.enabled))
      .catch((e) => {
        if (import.meta.env.DEV) console.warn('Failed to load setting:', e);
      })
      .finally(() => setLoading(false));
    api
      .fetchJson<{ value: number }>('/settings/max-cron-generation')
      .then((data) => setMaxCronGen(data.value))
      .catch((e) => {
        if (import.meta.env.DEV) console.warn('Failed to load setting:', e);
      });
  }, [api]);

  const handleToggle = async () => {
    const next = !yoloEnabled;
    try {
      await api.put('/settings/yolo', { enabled: next });
      setYoloEnabled(next);
    } catch (err) {
      if (import.meta.env.DEV) console.error('Failed to toggle YOLO mode:', err);
    }
  };

  const handleSetMaxCronGen = async (val: number) => {
    const clamped = Math.max(0, Math.min(6, val));
    try {
      await api.put('/settings/max-cron-generation', { value: clamped });
      setMaxCronGen(clamped);
    } catch (err) {
      if (import.meta.env.DEV) console.error('Failed to set max cron generation:', err);
    }
  };

  // Safe shutdown (Goal #164): drain MCP servers, stop the kernel, and — in the
  // desktop app — exit. The tray-menu Quit runs the identical sequence.
  const handleShutdown = async () => {
    setShutdownConfirm(false);
    showShutdownOverlay();
    try {
      if (isTauri) {
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke('shutdown_app');
      } else {
        await api.post('/system/shutdown', {});
      }
    } catch (err) {
      if (import.meta.env.DEV) console.error('Failed to shut down:', err);
    }
  };

  return (
    <>
      <SectionCard title={t('advanced.yolo_title')}>
        <div className="space-y-4">
          {!loading && (
            <Toggle enabled={yoloEnabled} onToggle={handleToggle} label={t('advanced.auto_approve_label')} />
          )}
          {yoloEnabled && (
            <div className="flex items-start gap-2 p-3 rounded-lg bg-amber-500/10 border border-amber-500/30">
              <AlertTriangle size={14} className="text-amber-400 mt-0.5 shrink-0" />
              <div className="space-y-1">
                <p className="text-xs font-bold text-amber-400 uppercase tracking-widest">
                  {t('advanced.yolo_warning')}
                </p>
              </div>
            </div>
          )}
          {!yoloEnabled && <p className="text-xs text-content-tertiary">{t('advanced.yolo_desc')}</p>}
        </div>
      </SectionCard>

      <SectionCard title={t('advanced.cron_limit_title')}>
        <div className="space-y-3">
          <p className="text-xs text-content-tertiary">{t('advanced.cron_limit_desc')}</p>
          <div className="flex items-center gap-3">
            <input
              type="number"
              min={0}
              max={6}
              value={maxCronGen}
              onChange={(e) => handleSetMaxCronGen(Number(e.target.value))}
              className="w-16 bg-surface-secondary border border-edge rounded px-2 py-1 text-xs font-mono text-content-primary"
            />
            <span className="text-xs text-content-tertiary">{t('advanced.cron_limit_hint')}</span>
          </div>
        </div>
      </SectionCard>

      <SectionCard title={t('advanced.shutdown_title')}>
        <div className="space-y-3">
          <p className="text-xs text-content-tertiary">{t('advanced.shutdown_desc')}</p>
          <button
            onClick={() => setShutdownConfirm(true)}
            className="flex items-center gap-2 px-3 py-1.5 rounded text-[10px] font-mono uppercase tracking-widest bg-red-500/20 text-red-400 hover:bg-red-500/30 transition-colors"
          >
            <Power size={12} />
            {t('advanced.shutdown_button')}
          </button>
        </div>
      </SectionCard>

      <ConfirmDialog
        open={shutdownConfirm}
        title={t('advanced.shutdown_confirm_title')}
        message={t('advanced.shutdown_confirm_message')}
        confirmLabel={t('advanced.shutdown_confirm_label')}
        variant="danger"
        onConfirm={handleShutdown}
        onCancel={() => setShutdownConfirm(false)}
      />
    </>
  );
}
