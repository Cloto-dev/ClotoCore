import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import LanguageDetector from 'i18next-browser-languagedetector';

import en_common from './locales/en/common.json';
import en_agents from './locales/en/agents.json';
import en_settings from './locales/en/settings.json';
import en_mcp from './locales/en/mcp.json';
import en_nav from './locales/en/nav.json';
import ja_common from './locales/ja/common.json';
import ja_agents from './locales/ja/agents.json';
import ja_settings from './locales/ja/settings.json';
import ja_mcp from './locales/ja/mcp.json';
import ja_nav from './locales/ja/nav.json';

const NAMESPACES = ['common', 'agents', 'settings', 'mcp', 'nav'] as const;
const CUSTOM_LANGS_KEY = 'cloto-custom-languages';

i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      en: { common: en_common, agents: en_agents, settings: en_settings, mcp: en_mcp, nav: en_nav },
      ja: { common: ja_common, agents: ja_agents, settings: ja_settings, mcp: ja_mcp, nav: ja_nav },
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

// Restore custom language packs from localStorage on startup
function restoreCustomLanguages() {
  try {
    const stored = localStorage.getItem(CUSTOM_LANGS_KEY);
    if (!stored) return;
    const packs: Record<string, { label: string; resources: Record<string, object> }> = JSON.parse(stored);
    for (const [code, pack] of Object.entries(packs)) {
      for (const ns of NAMESPACES) {
        if (pack.resources[ns]) {
          i18n.addResourceBundle(code, ns, pack.resources[ns], true, true);
        }
      }
    }
  } catch {
    // Silently ignore corrupt data
  }
}

restoreCustomLanguages();

/** Get list of custom language metadata from localStorage */
export function getCustomLanguages(): { code: string; label: string }[] {
  try {
    const stored = localStorage.getItem(CUSTOM_LANGS_KEY);
    if (!stored) return [];
    const packs = JSON.parse(stored);
    return Object.entries(packs).map(([code, pack]: [string, any]) => ({
      code,
      label: pack.label || code,
    }));
  } catch {
    return [];
  }
}

/** Export English locale as a translation template */
export function exportLanguageTemplate(): string {
  const template = {
    code: 'LANG_CODE',
    label: 'Language Name',
    common: en_common,
    agents: en_agents,
    settings: en_settings,
    mcp: en_mcp,
    nav: en_nav,
  };
  return JSON.stringify(template, null, 2);
}

/** Import a language pack JSON and register it with i18next */
export function importLanguagePack(json: string): { code: string; label: string } {
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

  // Persist to localStorage
  try {
    const stored = localStorage.getItem(CUSTOM_LANGS_KEY);
    const packs = stored ? JSON.parse(stored) : {};
    packs[code] = {
      label,
      resources: Object.fromEntries(
        NAMESPACES.filter(ns => pack[ns]).map(ns => [ns, pack[ns]])
      ),
    };
    localStorage.setItem(CUSTOM_LANGS_KEY, JSON.stringify(packs));
  } catch {
    // Storage full or unavailable — language still works for this session
  }

  return { code, label };
}

/** Remove a custom language pack */
export function removeCustomLanguage(code: string): void {
  // Remove from i18next
  for (const ns of NAMESPACES) {
    i18n.removeResourceBundle(code, ns);
  }

  // Remove from localStorage
  try {
    const stored = localStorage.getItem(CUSTOM_LANGS_KEY);
    if (stored) {
      const packs = JSON.parse(stored);
      delete packs[code];
      localStorage.setItem(CUSTOM_LANGS_KEY, JSON.stringify(packs));
    }
  } catch {
    // Ignore
  }

  // Switch to fallback if current language was removed
  if (i18n.language === code) {
    i18n.changeLanguage('en');
  }
}

export default i18n;
