import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useUserIdentity } from '../../contexts/UserIdentityContext';
import { useApi } from '../../hooks/useApi';
import { BUILTIN_LANGUAGES, exportLanguageTemplate, getCustomLanguages, importLanguagePack } from '../../i18n';
import { getLanguagesDir, isTauri, openFileDialog, readTextFile } from '../../lib/tauri';
import { Select, SettingsGroup, SettingsRow, Toggle } from './common';
import { ThemePackGroup, ThemeRows } from './ThemeSettings';

export function GeneralSection() {
  const api = useApi();
  const { identity, setIdentity } = useUserIdentity();
  const { t, i18n } = useTranslation('settings');
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [displayName, setDisplayName] = useState(identity.name);
  const [importStatus, setImportStatus] = useState<{ type: 'success' | 'error'; message: string } | null>(null);
  const [customLangs, setCustomLangs] = useState<{ code: string; label: string }[]>([]);
  const [injectLangEnabled, setInjectLangEnabled] = useState(true);
  const [injectLangLoaded, setInjectLangLoaded] = useState(false);

  // Load external languages from filesystem
  useEffect(() => {
    getCustomLanguages().then(setCustomLangs);
  }, []);

  // Load response-language injection setting from backend
  useEffect(() => {
    api
      .fetchJson<{ enabled: boolean; language: string }>('/settings/language')
      .then((data) => setInjectLangEnabled(data.enabled))
      .catch((e) => {
        if (import.meta.env.DEV) console.warn('Failed to load language setting:', e);
      })
      .finally(() => setInjectLangLoaded(true));
  }, [api]);

  // Sync UI language → backend so the system prompt uses the right code
  // when injection is enabled. Fire-and-forget; failure is non-fatal.
  const syncLanguageToBackend = (code: string) => {
    api.put('/settings/language', { language: code }).catch((e) => {
      if (import.meta.env.DEV) console.warn('Failed to sync language to backend:', e);
    });
  };

  const handleLanguageChange = (code: string) => {
    i18n.changeLanguage(code);
    syncLanguageToBackend(code);
  };

  const handleToggleInjectLang = async () => {
    const next = !injectLangEnabled;
    try {
      await api.put('/settings/language', { enabled: next });
      setInjectLangEnabled(next);
    } catch (err) {
      if (import.meta.env.DEV) console.error('Failed to toggle language injection:', err);
    }
  };

  const builtinCodes = new Set(BUILTIN_LANGUAGES.map((l) => l.code));

  const allLanguages = [
    ...BUILTIN_LANGUAGES,
    ...customLangs.filter((l) => !builtinCodes.has(l.code)).map((l) => ({ ...l, custom: true })),
  ];

  const handleExport = () => {
    const json = exportLanguageTemplate();
    const blob = new Blob([json], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'cloto-language-template.json';
    a.click();
    URL.revokeObjectURL(url);
  };

  const processImportJson = async (json: string) => {
    try {
      const result = await importLanguagePack(json);
      const langs = await getCustomLanguages();
      setCustomLangs(langs);
      i18n.changeLanguage(result.code);
      syncLanguageToBackend(result.code);
      setImportStatus({
        type: 'success',
        message: t('general.import_success', { label: result.label, code: result.code }),
      });
    } catch (err: unknown) {
      const errMessage = err instanceof Error ? err.message : 'Unknown import error';
      setImportStatus({
        type: 'error',
        message: t('general.import_error', { error: errMessage }),
      });
    }
  };

  const handleImportClick = async () => {
    setImportStatus(null);

    if (isTauri) {
      // Native dialog with default path to Documents/ClotoCore/languages
      const defaultPath = (await getLanguagesDir()) ?? undefined;
      const filePath = await openFileDialog({
        title: t('general.import_pack'),
        defaultPath,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      if (!filePath) return;
      const content = await readTextFile(filePath);
      if (content) processImportJson(content);
    } else {
      // Browser fallback: trigger hidden file input
      fileInputRef.current?.click();
    }
  };

  const handleImport = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setImportStatus(null);

    const reader = new FileReader();
    reader.onload = () => processImportJson(reader.result as string);
    reader.readAsText(file);

    // Reset input so same file can be re-imported
    e.target.value = '';
  };

  return (
    <>
      <SettingsGroup title={t('general.group_display')}>
        <ThemeRows />

        <SettingsRow label={t('general.language')} desc={t('general.language_desc')}>
          <Select
            label={t('general.language')}
            value={i18n.language.split('-')[0]}
            onChange={handleLanguageChange}
            options={allLanguages.map((lang) => ({
              value: lang.code,
              label: 'custom' in lang ? `${lang.label} (${t('general.custom_label')})` : lang.label,
            }))}
          />
        </SettingsRow>

        {injectLangLoaded && (
          <SettingsRow label={t('general.inject_language_to_prompt')} desc={t('general.inject_language_hint')}>
            <Toggle
              label={t('general.inject_language_to_prompt')}
              checked={injectLangEnabled}
              onChange={handleToggleInjectLang}
            />
          </SettingsRow>
        )}
      </SettingsGroup>

      <SettingsGroup title={t('general.group_user')}>
        <SettingsRow label={t('general.display_name')} desc={t('general.name_hint')}>
          <input
            className="in"
            type="text"
            aria-label={t('general.display_name')}
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
            onBlur={() => setIdentity(identity.id, displayName)}
            placeholder={t('general.name_placeholder')}
          />
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup title={t('general.group_language_pack')}>
        <SettingsRow label={t('general.pack_label')} desc={t('general.pack_desc')}>
          <button type="button" className="btn" onClick={handleExport}>
            {t('general.export_template')}
          </button>
          <button type="button" className="btn" onClick={handleImportClick}>
            {t('general.import_pack')}
          </button>
          <input ref={fileInputRef} type="file" accept=".json" onChange={handleImport} className="hidden" />
        </SettingsRow>
        {importStatus && (
          <p className={importStatus.type === 'success' ? 'says ok' : 'says bad'}>{importStatus.message}</p>
        )}
      </SettingsGroup>

      <ThemePackGroup />
    </>
  );
}
