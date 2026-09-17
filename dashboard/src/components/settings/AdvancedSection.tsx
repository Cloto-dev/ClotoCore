import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { SettingsGroup, SettingsRow, Toggle } from './common';

export function AdvancedSection() {
  const api = useApi();
  const { t } = useTranslation('settings');
  const [yoloEnabled, setYoloEnabled] = useState(false);
  const [loading, setLoading] = useState(true);
  const [maxCronGen, setMaxCronGen] = useState(2);

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

  return (
    <>
      <SettingsGroup title={t('advanced.yolo_title')}>
        {!loading && (
          <SettingsRow label={t('advanced.auto_approve_label')} desc={t('advanced.yolo_desc')}>
            <Toggle label={t('advanced.auto_approve_label')} checked={yoloEnabled} onChange={handleToggle} />
          </SettingsRow>
        )}
        {yoloEnabled && <p className="says warn">{t('advanced.yolo_warning')}</p>}
      </SettingsGroup>

      <SettingsGroup title={t('advanced.cron_limit_title')}>
        <SettingsRow label={t('advanced.cron_limit_label')} desc={t('advanced.cron_limit_desc')}>
          <input
            className="in tiny num"
            type="number"
            min={0}
            max={6}
            aria-label={t('advanced.cron_limit_label')}
            value={maxCronGen}
            onChange={(e) => handleSetMaxCronGen(Number(e.target.value))}
          />
          <span className="val">{t('advanced.cron_limit_hint')}</span>
        </SettingsRow>
      </SettingsGroup>
    </>
  );
}
