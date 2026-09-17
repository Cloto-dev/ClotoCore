import { useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { type ThemeMode, useTheme } from '../../hooks/useTheme';
import { isTauri, openFileDialog, readTextFile } from '../../lib/tauri';
import { exportThemeTemplate } from '../../themes/load';
import { Segmented, Select, SettingsGroup, SettingsRow } from './common';

const MODES: ThemeMode[] = ['light', 'dark', 'system'];

/** The theme and mode rows of the display group. The list is whatever the
 * loader found — nothing here names a theme (docs/THEME_PACKS_DESIGN.md §2 (b)). */
export function ThemeRows() {
  const { t } = useTranslation('settings');
  const { themes, themeId, setThemeId, mode, setMode } = useTheme();
  const current = themes.find((th) => th.theme.id === themeId);
  const faces = current ? Object.keys(current.theme.faces) : [];
  const oneFace = faces.length === 1;

  return (
    <>
      <SettingsRow
        label={t('general.theme')}
        desc={
          current && current.warnings.length > 0 ? t('general.theme_low_contrast_desc') : t('general.theme_pick_desc')
        }
      >
        <Select
          label={t('general.theme')}
          value={themeId}
          onChange={setThemeId}
          options={themes.map(({ theme, warnings }) => ({
            value: theme.id,
            label: warnings.length > 0 ? t('general.theme_low_contrast', { label: theme.label }) : theme.label,
          }))}
        />
      </SettingsRow>
      <SettingsRow
        label={t('general.theme_mode')}
        desc={oneFace ? t(`general.theme_mode_only_${faces[0]}`) : t('general.theme_desc')}
      >
        <Segmented<ThemeMode>
          label={t('general.theme_mode')}
          value={mode}
          onChange={setMode}
          options={MODES.map((m) => ({ value: m, label: t(`general.theme_${m}`) }))}
        />
      </SettingsRow>
    </>
  );
}

/** Import, export a template, and remove external theme packs. */
export function ThemePackGroup() {
  const { t } = useTranslation('settings');
  const { themes, rejected, importPack, removePack } = useTheme();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [status, setStatus] = useState<{ type: 'success' | 'error'; message: string } | null>(null);
  const external = themes.filter((th) => th.source !== 'bundled');

  const take = async (json: string) => {
    try {
      const loaded = await importPack(json);
      setStatus({ type: 'success', message: t('general.theme_import_success', { label: loaded.theme.label }) });
    } catch (err: unknown) {
      const reason = err instanceof Error ? err.message : String(err);
      setStatus({ type: 'error', message: t('general.theme_import_error', { error: reason }) });
    }
  };

  const handleImportClick = async () => {
    setStatus(null);
    if (!isTauri) {
      fileInputRef.current?.click();
      return;
    }
    const filePath = await openFileDialog({
      title: t('general.theme_import'),
      filters: [{ name: 'JSON', extensions: ['json'] }],
    });
    if (!filePath) return;
    const content = await readTextFile(filePath);
    if (content) await take(content);
  };

  const handleFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setStatus(null);
    const reader = new FileReader();
    reader.onload = () => void take(reader.result as string);
    reader.readAsText(file);
    // Reset so the same file can be chosen again after it is fixed.
    e.target.value = '';
  };

  const handleExport = () => {
    const blob = new Blob([exportThemeTemplate()], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'cloto-theme-template.json';
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <SettingsGroup title={t('general.group_theme_pack')}>
      <SettingsRow label={t('general.theme_pack_label')} desc={t('general.theme_pack_desc')}>
        <button type="button" className="btn" onClick={handleExport}>
          {t('general.export_template')}
        </button>
        <button type="button" className="btn" onClick={handleImportClick}>
          {t('general.theme_import')}
        </button>
        <input ref={fileInputRef} type="file" accept=".json" onChange={handleFile} className="hidden" />
      </SettingsRow>
      {external.map(({ theme }) => (
        <SettingsRow
          key={theme.id}
          label={theme.label}
          desc={theme.author ? t('general.theme_by', { author: theme.author }) : theme.id}
        >
          <button type="button" className="btn" onClick={() => void removePack(theme.id)}>
            {t('general.theme_remove')}
          </button>
        </SettingsRow>
      ))}
      {rejected.map((pack) => (
        <p key={pack.name} className="says bad">
          {t('general.theme_rejected', { name: pack.name, error: pack.errors.join('; ') })}
        </p>
      ))}
      {status && <p className={status.type === 'success' ? 'says ok' : 'says bad'}>{status.message}</p>}
    </SettingsGroup>
  );
}
