import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import LanguageDetector from 'i18next-browser-languagedetector';
import {
  installDefaultPacks,
  scanLanguagesDir,
  saveLanguagePack as savePack,
  removeLanguagePack as removePack,
} from './lib/tauri';

// Bundled: English only
import en_common from './locales/en/common.json';
import en_agents from './locales/en/agents.json';
import en_settings from './locales/en/settings.json';
import en_mcp from './locales/en/mcp.json';
import en_nav from './locales/en/nav.json';
import en_cron from './locales/en/cron.json';
import en_memory from './locales/en/memory.json';
import en_wizard from './locales/en/wizard.json';

const NAMESPACES = ['common', 'agents', 'settings', 'mcp', 'nav', 'cron', 'memory', 'wizard'] as const;

i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      en: { common: en_common, agents: en_agents, settings: en_settings, mcp: en_mcp, nav: en_nav, cron: en_cron, memory: en_memory, wizard: en_wizard },
    },
    fallbackLng: 'en',
    defaultNS: 'common',
    interpolation: { escapeValue: false },
    detection: {
      order: ['localStorage', 'navigator'],
      caches: ['localStorage'],
      lookupLocalStorage: 'cloto-language',
    },
  });

/**
 * Load all external language packs from Documents/ClotoCore/languages/.
 * Must be called (and awaited) before React renders.
 */
export async function loadExternalLanguages(): Promise<void> {
  // Ensure bundled default packs are installed on first run
  await installDefaultPacks();

  const packs = await scanLanguagesDir();
  const loadedCodes = new Set<string>();
  for (const [, content] of packs) {
    try {
      const pack = JSON.parse(content);
      if (!pack.code || typeof pack.code !== 'string') continue;
      for (const ns of NAMESPACES) {
        if (pack[ns] && typeof pack[ns] === 'object') {
          i18n.addResourceBundle(pack.code, ns, pack[ns], true, true);
        }
      }
      loadedCodes.add(pack.code);
    } catch {
      // Skip invalid JSON files
    }
  }

  // First launch: auto-detect browser language and apply if a matching pack exists.
  // LanguageDetector may have fallen back to 'en' at init time because external
  // packs weren't loaded yet. Re-apply the detected language now that packs are available.
  if (!localStorage.getItem('cloto-language')) {
    const browserLang = navigator.language.split('-')[0];
    if (browserLang !== 'en' && (loadedCodes.has(browserLang) || i18n.hasResourceBundle(browserLang, 'common'))) {
      i18n.changeLanguage(browserLang);
    }
  }
}

/** Get list of external language packs (filesystem-based). */
export async function getCustomLanguages(): Promise<{ code: string; label: string }[]> {
  const packs = await scanLanguagesDir();
  const result: { code: string; label: string }[] = [];
  for (const [, content] of packs) {
    try {
      const pack = JSON.parse(content);
      if (pack.code && pack.label) {
        result.push({ code: pack.code, label: pack.label });
      }
    } catch {
      // Skip invalid
    }
  }
  return result;
}

/** Export English locale as a translation template. */
export function exportLanguageTemplate(): string {
  const template = {
    code: 'LANG_CODE',
    label: 'Language Name',
    common: en_common,
    agents: en_agents,
    settings: en_settings,
    mcp: en_mcp,
    nav: en_nav,
    cron: en_cron,
    memory: en_memory,
    wizard: en_wizard,
  };
  return JSON.stringify(template, null, 2);
}

/** Import a language pack JSON, register with i18next, and save to filesystem. */
export async function importLanguagePack(json: string): Promise<{ code: string; label: string }> {
  const pack = JSON.parse(json);

  if (!pack.code || typeof pack.code !== 'string') {
    throw new Error('Missing "code" field (e.g. "pt-BR")');
  }
  if (!pack.label || typeof pack.label !== 'string') {
    throw new Error('Missing "label" field (e.g. "Português")');
  }

  const code: string = pack.code;
  const label: string = pack.label;

  // Register each namespace with i18next
  for (const ns of NAMESPACES) {
    if (pack[ns] && typeof pack[ns] === 'object') {
      i18n.addResourceBundle(code, ns, pack[ns], true, true);
    }
  }

  // Persist to filesystem
  await savePack(code, json);

  return { code, label };
}

/** Remove a custom language pack (filesystem + i18next). */
export async function removeCustomLanguage(code: string): Promise<void> {
  // Remove from i18next
  for (const ns of NAMESPACES) {
    i18n.removeResourceBundle(code, ns);
  }

  // Remove from filesystem
  await removePack(code);

  // Switch to fallback if current language was removed
  if (i18n.language === code) {
    i18n.changeLanguage('en');
  }
}

export default i18n;
