import { createContext, useContext, useEffect, useState, useCallback } from 'react';

type Theme = 'light' | 'dark';
type ThemePreference = 'light' | 'dark' | 'system';

interface ThemeContextValue {
  theme: Theme;
  preference: ThemePreference;
  setPreference: (pref: ThemePreference) => void;
  toggle: () => void;
  colors: {
    brandHex: string;
    canvasBg: string;
    canvasGrid: string;
    canvasNodeFill: string;
    canvasText: string;
  };
}

const STORAGE_KEY = 'exiv-theme';

function getSystemTheme(): Theme {
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function resolveTheme(pref: ThemePreference): Theme {
  return pref === 'system' ? getSystemTheme() : pref;
}

function getCanvasColors(theme: Theme) {
  return theme === 'dark'
    ? { brandHex: '#5b7aff', canvasBg: '#0f172a', canvasGrid: '#334155', canvasNodeFill: '#1e293b', canvasText: 'rgba(226,232,240,0.8)' }
    : { brandHex: '#2e4de6', canvasBg: '#f8fafc', canvasGrid: '#cbd5e1', canvasNodeFill: '#ffffff', canvasText: 'rgba(15,23,42,0.8)' };
}

export const ThemeContext = createContext<ThemeContextValue | null>(null);

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error('useTheme must be used within ThemeProvider');
  return ctx;
}

export function useThemeProvider() {
  const stored = localStorage.getItem(STORAGE_KEY) as ThemePreference | null;
  const [preference, setPreferenceState] = useState<ThemePreference>(stored || 'system');
  const [theme, setTheme] = useState<Theme>(() => resolveTheme(stored || 'system'));

  const applyTheme = useCallback((t: Theme) => {
    document.documentElement.classList.toggle('dark', t === 'dark');
    setTheme(t);
  }, []);

  const setPreference = useCallback((pref: ThemePreference) => {
    setPreferenceState(pref);
    localStorage.setItem(STORAGE_KEY, pref);
    applyTheme(resolveTheme(pref));
  }, [applyTheme]);

  const toggle = useCallback(() => {
    setPreference(theme === 'light' ? 'dark' : 'light');
  }, [theme, setPreference]);

  useEffect(() => {
    applyTheme(resolveTheme(preference));
  }, []);

  useEffect(() => {
    if (preference !== 'system') return;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = () => applyTheme(getSystemTheme());
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, [preference, applyTheme]);

  return {
    theme,
    preference,
    setPreference,
    toggle,
    colors: getCanvasColors(theme),
  };
}
