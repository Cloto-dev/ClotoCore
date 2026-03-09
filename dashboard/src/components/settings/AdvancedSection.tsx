import { useState, useEffect } from 'react';
import { AlertTriangle } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { SectionCard, Toggle } from './common';
import { useApi } from '../../hooks/useApi';

export function AdvancedSection() {
  const api = useApi();
  const { t } = useTranslation('settings');
  const [yoloEnabled, setYoloEnabled] = useState(false);
  const [loading, setLoading] = useState(true);
  const [maxCronGen, setMaxCronGen] = useState(2);

  useEffect(() => {
    api.fetchJson<{ enabled: boolean }>('/settings/yolo')
      .then(data => setYoloEnabled(data.enabled))
      .catch(() => {})
      .finally(() => setLoading(false));
    api.fetchJson<{ value: number }>('/settings/max-cron-generation')
      .then(data => setMaxCronGen(data.value))
      .catch(() => {});
  }, [api]);

  const handleToggle = async () => {
    const next = !yoloEnabled;
    try {
      await api.put('/settings/yolo', { enabled: next });
      setYoloEnabled(next);
    } catch (err) {
      console.error('Failed to toggle YOLO mode:', err);
    }
  };

  const handleSetMaxCronGen = async (val: number) => {
    const clamped = Math.max(0, Math.min(6, val));
    try {
      await api.put('/settings/max-cron-generation', { value: clamped });
      setMaxCronGen(clamped);
    } catch (err) {
      console.error('Failed to set max cron generation:', err);
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
                <p className="text-xs font-bold text-amber-400 uppercase tracking-widest">{t('advanced.yolo_warning')}</p>
              </div>
            </div>
          )}
          {!yoloEnabled && (
            <p className="text-xs text-content-tertiary">{t('advanced.yolo_desc')}</p>
          )}
        </div>
      </SectionCard>

      <SectionCard title={t('advanced.cron_limit_title')}>
        <div className="space-y-3">
          <p className="text-xs text-content-tertiary">
            {t('advanced.cron_limit_desc')}
          </p>
          <div className="flex items-center gap-3">
            <input
              type="number"
              min={0}
              max={6}
              value={maxCronGen}
              onChange={e => handleSetMaxCronGen(Number(e.target.value))}
              className="w-16 bg-surface-secondary border border-edge rounded px-2 py-1 text-xs font-mono text-content-primary"
            />
            <span className="text-xs text-content-tertiary">{t('advanced.cron_limit_hint')}</span>
          </div>
        </div>
      </SectionCard>
    </>
  );
}
