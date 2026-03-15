import { Download, Globe, Monitor, Moon, Sun, Upload } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useUserIdentity } from '../../contexts/UserIdentityContext';
import { useTheme } from '../../hooks/useTheme';
import { exportLanguageTemplate, getCustomLanguages, importLanguagePack } from '../../i18n';
import { getLanguagesDir, isTauri, openFileDialog, readTextFile } from '../../lib/tauri';
import { SectionCard } from './common';

const BUILTIN_LANGUAGES = [
  { code: 'en', label: 'English' },
  { code: 'ja', label: '日本語' },
];

export function GeneralSection() {
  const { preference, setPreference } = useTheme();
  const { identity, setIdentity } = useUserIdentity();
  const { t, i18n } = useTranslation('settings');
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [displayName, setDisplayName] = useState(identity.name);
  const [importStatus, setImportStatus] = useState<{ type: 'success' | 'error'; message: string } | null>(null);
  const [customLangs, setCustomLangs] = useState<{ code: string; label: string }[]>([]);

  // Load external languages from filesystem
  useEffect(() => {
    getCustomLanguages().then(setCustomLangs);
  }, []);

  const builtinCodes = new Set(BUILTIN_LANGUAGES.map((l) => l.code));

  const allLanguages = [
    ...BUILTIN_LANGUAGES,
    ...customLangs.filter((l) => !builtinCodes.has(l.code)).map((l) => ({ ...l, custom: true })),
  ];

  const themes: { value: 'light' | 'dark' | 'system'; icon: typeof Sun; labelKey: string }[] = [
    { value: 'light', icon: Sun, labelKey: 'general.theme_light' },
    { value: 'dark', icon: Moon, labelKey: 'general.theme_dark' },
    { value: 'system', icon: Monitor, labelKey: 'general.theme_system' },
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
      setImportStatus({
        type: 'success',
        message: t('general.import_success', { label: result.label, code: result.code }),
      });
    } catch (err: any) {
      setImportStatus({
        type: 'error',
        message: t('general.import_error', { error: err.message }),
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
      <SectionCard title={t('general.theme')}>
        <div className="flex gap-3">
          {themes.map(({ value, icon: Icon, labelKey }) => (
            <button
              key={value}
              onClick={() => setPreference(value)}
              className={`flex items-center gap-2 px-5 py-2.5 rounded-xl text-xs font-bold transition-all ${
                preference === value
                  ? 'bg-brand text-white shadow-md'
                  : 'bg-surface-secondary text-content-secondary hover:text-content-primary border border-edge hover:border-brand'
              }`}
            >
              <Icon size={14} />
              {t(labelKey)}
            </button>
          ))}
        </div>
      </SectionCard>

      <SectionCard title={t('general.language')}>
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <Globe size={14} className="text-content-tertiary shrink-0" />
            <select
              value={i18n.language.split('-')[0]}
              onChange={(e) => i18n.changeLanguage(e.target.value)}
              className="px-3 py-2 bg-surface-secondary border border-edge rounded-lg text-sm text-content-primary focus:border-brand focus:outline-none transition-colors"
            >
              {allLanguages.map((lang) => (
                <option key={lang.code} value={lang.code}>
                  {lang.label}
                  {'custom' in lang ? ` (${t('general.custom_label')})` : ''}
                </option>
              ))}
            </select>
          </div>

          {/* Import / Export buttons */}
          <div className="flex items-center gap-2">
            <button
              onClick={handleExport}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-edge text-xs font-bold text-content-tertiary hover:text-brand hover:border-brand transition-all"
            >
              <Download size={12} />
              {t('general.export_template')}
            </button>
            <button
              onClick={handleImportClick}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-edge text-xs font-bold text-content-tertiary hover:text-brand hover:border-brand transition-all"
            >
              <Upload size={12} />
              {t('general.import_pack')}
            </button>
            <input ref={fileInputRef} type="file" accept=".json" onChange={handleImport} className="hidden" />
          </div>

          {/* Import status message */}
          {importStatus && (
            <p className={`text-xs ${importStatus.type === 'success' ? 'text-emerald-500' : 'text-red-400'}`}>
              {importStatus.message}
            </p>
          )}
        </div>
      </SectionCard>

      <SectionCard title={t('general.user_identity')}>
        <div className="space-y-3">
          <div>
            <label className="text-xs text-content-tertiary font-bold uppercase tracking-widest block mb-1">
              {t('general.display_name')}
            </label>
            <input
              type="text"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              onBlur={() => setIdentity(identity.id, displayName)}
              className="w-full px-3 py-2 bg-surface-secondary border border-edge rounded-lg text-sm text-content-primary focus:border-brand focus:outline-none transition-colors"
              placeholder={t('general.name_placeholder')}
            />
          </div>
          <p className="text-xs text-content-tertiary">{t('general.name_hint')}</p>
        </div>
      </SectionCard>
    </>
  );
}
