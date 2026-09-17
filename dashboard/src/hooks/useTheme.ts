import { createContext, useCallback, useContext, useEffect, useState } from 'react';
import { setWindowTheme } from '../lib/tauri';

type Theme = 'light' | 'dark';
/** `legacy` is the palette from before the redesign; like `system`, it follows the
 * OS between light and dark (see `.theme-legacy` in index.css). */
export type ThemePreference = 'light' | 'dark' | 'system' | 'legacy';

interface ThemeContextValue {
  theme: Theme;
  preference: ThemePreference;
  setPreference: (pref: ThemePreference) => void;
  toggle: () => void;
}

const STORAGE_KEY = 'cloto-theme';

function getSystemTheme(): Theme {
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function followsSystem(pref: ThemePreference): boolean {
  return pref === 'system' || pref === 'legacy';
}

function resolveTheme(pref: ThemePreference): Theme {
  return followsSystem(pref) ? getSystemTheme() : (pref as Theme);
}

export const ThemeContext = createContext<ThemeContextValue | null>(null);

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error('useTheme must be used within ThemeProvider');
  return ctx;
}

export function useThemeProvider() {
  const stored = localStorage.getItem(STORAGE_KEY) as ThemePreference | null;
  // Dark unless the user chose otherwise (docs/DESIGN_PHILOSOPHY.md §4.1).
  const [preference, setPreferenceState] = useState<ThemePreference>(stored || 'dark');
  const [theme, setTheme] = useState<Theme>(() => resolveTheme(stored || 'dark'));

  const applyTheme = useCallback((t: Theme, pref: ThemePreference) => {
    document.documentElement.classList.toggle('dark', t === 'dark');
    document.documentElement.classList.toggle('theme-legacy', pref === 'legacy');
    // The window's frame is the OS's; it follows the app's theme, not the system's.
    void setWindowTheme(t);
    setTheme(t);
  }, []);

  const setPreference = useCallback(
    (pref: ThemePreference) => {
      setPreferenceState(pref);
      localStorage.setItem(STORAGE_KEY, pref);
      applyTheme(resolveTheme(pref), pref);
    },
    [applyTheme],
  );

  const toggle = useCallback(() => {
    setPreference(theme === 'light' ? 'dark' : 'light');
  }, [theme, setPreference]);

  useEffect(() => {
    applyTheme(resolveTheme(preference), preference);
  }, [preference, applyTheme]);

  useEffect(() => {
    if (!followsSystem(preference)) return;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = () => applyTheme(getSystemTheme(), preference);
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, [preference, applyTheme]);

  return {
    theme,
    preference,
    setPreference,
    toggle,
  };
}
