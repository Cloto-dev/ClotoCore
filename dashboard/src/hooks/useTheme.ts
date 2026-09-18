import { createContext, useCallback, useContext, useEffect, useState } from 'react';
import { setWindowTheme } from '../lib/tauri';
import { applyTheme, type ThemeMode } from '../themes/apply';
import {
  FALLBACK_THEME_ID,
  findTheme,
  getRejectedPacks,
  getThemes,
  importThemePack,
  type LoadedTheme,
  type RejectedPack,
  removeThemePack,
} from '../themes/load';
import type { FaceName } from '../themes/validate';

export type { ThemeMode } from '../themes/apply';

interface ThemeContextValue {
  /** The face on screen. */
  face: FaceName;
  /** Which face to draw: pinned, or the OS's. A one-face theme ignores it. */
  mode: ThemeMode;
  setMode: (mode: ThemeMode) => void;
  /** The pack on screen. */
  themeId: string;
  setThemeId: (id: string) => void;
  themes: LoadedTheme[];
  rejected: RejectedPack[];
  /** Validate, keep and switch to a pack. Rejects with the reasons. */
  importPack: (json: string) => Promise<LoadedTheme>;
  removePack: (id: string) => Promise<void>;
}

export const THEME_ID_KEY = 'cloto-theme-id';
export const THEME_MODE_KEY = 'cloto-theme-mode';
/** The single setting that came before theme and mode were two. */
const OLD_PREFERENCE_KEY = 'cloto-theme';

const MODES: readonly ThemeMode[] = ['light', 'dark', 'system'];

function osIsDark(): boolean {
  return window.matchMedia('(prefers-color-scheme: dark)').matches;
}

/**
 * The stored theme and mode. The old single setting held a mode, or the id of a
 * bundled theme that followed the OS; it is read once and split in two.
 */
export function readStoredTheme(): { themeId: string; mode: ThemeMode } {
  const storedMode = localStorage.getItem(THEME_MODE_KEY);
  const storedId = localStorage.getItem(THEME_ID_KEY);
  if (storedMode !== null || storedId !== null) {
    return {
      themeId: storedId ?? FALLBACK_THEME_ID,
      // Dark unless the user chose otherwise (docs/DESIGN_PHILOSOPHY.md §4.1).
      mode: MODES.includes(storedMode as ThemeMode) ? (storedMode as ThemeMode) : 'dark',
    };
  }
  const old = localStorage.getItem(OLD_PREFERENCE_KEY);
  if (old === null) return { themeId: FALLBACK_THEME_ID, mode: 'dark' };
  if (MODES.includes(old as ThemeMode)) return { themeId: FALLBACK_THEME_ID, mode: old as ThemeMode };
  return { themeId: old, mode: 'system' };
}

function persist(themeId: string, mode: ThemeMode) {
  localStorage.setItem(THEME_ID_KEY, themeId);
  localStorage.setItem(THEME_MODE_KEY, mode);
  localStorage.removeItem(OLD_PREFERENCE_KEY);
}

/** Apply the stored theme. `main.tsx` calls it before React renders, so the
 * accent flag and surface are in place before any agent's colour is computed. */
export function applyStoredTheme(): FaceName {
  const { themeId, mode } = readStoredTheme();
  return applyTheme(findTheme(themeId).theme, mode, osIsDark());
}

export const ThemeContext = createContext<ThemeContextValue | null>(null);

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error('useTheme must be used within ThemeProvider');
  return ctx;
}

export function useThemeProvider(): ThemeContextValue {
  const [stored] = useState(readStoredTheme);
  // An id whose pack is gone draws the fallback, and says so in the picker.
  const [themeId, setThemeIdState] = useState(() => findTheme(stored.themeId).theme.id);
  const [mode, setModeState] = useState<ThemeMode>(stored.mode);
  const [themes, setThemes] = useState(getThemes);
  const [face, setFace] = useState<FaceName>('dark');

  const apply = useCallback((id: string, m: ThemeMode) => {
    const drawn = applyTheme(findTheme(id).theme, m, osIsDark());
    // The window's frame is the OS's; it follows the app's face, not the system's.
    void setWindowTheme(drawn);
    setFace(drawn);
  }, []);

  useEffect(() => {
    apply(themeId, mode);
    persist(themeId, mode);
  }, [themeId, mode, apply]);

  useEffect(() => {
    if (mode !== 'system') return;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = () => apply(themeId, mode);
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, [themeId, mode, apply]);

  const importPack = useCallback(async (json: string) => {
    const loaded = await importThemePack(json);
    setThemes(getThemes());
    setThemeIdState(loaded.theme.id);
    return loaded;
  }, []);

  const removePack = useCallback(async (id: string) => {
    await removeThemePack(id);
    setThemes(getThemes());
    setThemeIdState((current) => (current === id ? FALLBACK_THEME_ID : current));
  }, []);

  return {
    face,
    mode,
    setMode: setModeState,
    themeId,
    setThemeId: setThemeIdState,
    themes,
    rejected: getRejectedPacks(),
    importPack,
    removePack,
  };
}
