import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const tauri = vi.hoisted(() => ({
  setWindowTheme: vi.fn(async () => {}),
  scanThemesDir: vi.fn(async (): Promise<Array<[string, string]>> => []),
  saveThemePack: vi.fn(async () => false),
  removeThemePack: vi.fn(async () => false),
}));
vi.mock('../../lib/tauri', async (original) => ({ ...(await original<typeof import('../../lib/tauri')>()), ...tauri }));

import { STYLE_ELEMENT_ID } from '../../themes/apply';
import { applyStoredTheme, readStoredTheme, THEME_ID_KEY, THEME_MODE_KEY, useThemeProvider } from '../useTheme';

const NIGHT = JSON.stringify({
  schema: 1,
  id: 'night',
  label: 'Night',
  dark: {
    'surface-base': '260 20% 6%',
    'surface-secondary': '260 20% 10%',
    'surface-primary': '260 20% 14%',
    'border-default': '260 20% 22%',
    'border-subtle': '260 20% 14%',
    'surface-overlay': '0 0% 0% / 0.6',
    'text-primary': '260 10% 94%',
    'text-secondary': '260 8% 72%',
    'text-tertiary': '260 6% 64%',
    'text-muted': '260 6% 40%',
  },
});

function stubOsDark(dark: boolean) {
  vi.stubGlobal(
    'matchMedia',
    vi.fn().mockImplementation(() => ({
      matches: dark,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
}

const root = () => document.documentElement;
const stylesheet = () => document.getElementById(STYLE_ELEMENT_ID)?.textContent ?? '';

afterEach(() => {
  localStorage.clear();
  root().className = '';
  delete root().dataset.accent;
  document.getElementById(STYLE_ELEMENT_ID)?.remove();
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

describe('the stored theme', () => {
  it('is the fallback theme in dark when nothing was chosen, whatever the OS says', () => {
    stubOsDark(false);
    const { result } = renderHook(() => useThemeProvider());
    expect(result.current.themeId).toBe('default');
    expect(result.current.mode).toBe('dark');
    expect(result.current.face).toBe('dark');
    expect(root().classList.contains('dark')).toBe(true);
  });

  it('splits the old single setting in two: a mode stays a mode on the fallback theme', () => {
    localStorage.setItem('cloto-theme', 'light');
    expect(readStoredTheme()).toEqual({ themeId: 'default', mode: 'light' });
    localStorage.setItem('cloto-theme', 'system');
    expect(readStoredTheme()).toEqual({ themeId: 'default', mode: 'system' });
  });

  it('splits the old single setting in two: a theme name becomes that theme, following the OS', () => {
    // Read from the bundled packs rather than spelled here: the old setting held
    // the id of the one bundled theme that is not the fallback.
    stubOsDark(true);
    const { result: first } = renderHook(() => useThemeProvider());
    const other = first.current.themes.map((t) => t.theme.id).find((id) => id !== 'default') ?? '';
    expect(other).not.toBe('');
    localStorage.clear();

    localStorage.setItem('cloto-theme', other);
    expect(readStoredTheme()).toEqual({ themeId: other, mode: 'system' });
    const { result } = renderHook(() => useThemeProvider());
    expect(result.current.themeId).toBe(other);
    expect(result.current.face).toBe('dark');
    // Written back in the new form, and the old key is gone.
    expect(localStorage.getItem(THEME_ID_KEY)).toBe(other);
    expect(localStorage.getItem(THEME_MODE_KEY)).toBe('system');
    expect(localStorage.getItem('cloto-theme')).toBeNull();
  });

  it('draws the fallback for a stored id whose pack is gone', () => {
    stubOsDark(false);
    localStorage.setItem(THEME_ID_KEY, 'removed-long-ago');
    localStorage.setItem(THEME_MODE_KEY, 'light');
    const { result } = renderHook(() => useThemeProvider());
    expect(result.current.themeId).toBe('default');
    expect(result.current.face).toBe('light');
  });

  it('is applied before React by applyStoredTheme, flag and stylesheet included', () => {
    stubOsDark(false);
    localStorage.setItem(THEME_MODE_KEY, 'light');
    expect(applyStoredTheme()).toBe('light');
    expect(root().classList.contains('dark')).toBe(false);
    expect(stylesheet()).toContain('--surface-base: 190 6% 92%;');
  });
});

describe('changing the theme', () => {
  it('switches mode and theme independently, and tells the window frame the face', () => {
    stubOsDark(false);
    const { result } = renderHook(() => useThemeProvider());
    const other = result.current.themes.map((t) => t.theme.id).find((id) => id !== 'default') ?? '';

    act(() => result.current.setMode('light'));
    expect(result.current.face).toBe('light');
    expect(root().classList.contains('dark')).toBe(false);
    expect(tauri.setWindowTheme).toHaveBeenLastCalledWith('light');

    act(() => result.current.setThemeId(other));
    expect(result.current.mode).toBe('light');
    expect(root().dataset.accent).toBe('fixed');
    expect(localStorage.getItem(THEME_ID_KEY)).toBe(other);

    act(() => result.current.setThemeId('default'));
    expect(root().dataset.accent).toBeUndefined();
    expect(stylesheet()).toContain('--surface-base: 190 6% 92%;');
  });

  it('follows the OS under system', () => {
    stubOsDark(true);
    const { result } = renderHook(() => useThemeProvider());
    act(() => result.current.setMode('system'));
    expect(result.current.face).toBe('dark');
  });

  it('imports a pack, switches to it, and draws its only face whatever the mode says', async () => {
    stubOsDark(false);
    const { result } = renderHook(() => useThemeProvider());
    act(() => result.current.setMode('light'));
    await act(async () => {
      await result.current.importPack(NIGHT);
    });
    expect(result.current.themeId).toBe('night');
    expect(result.current.themes.map((t) => t.theme.id)).toContain('night');
    expect(result.current.face).toBe('dark');
    expect(root().classList.contains('dark')).toBe(true);

    await act(async () => {
      await result.current.removePack('night');
    });
    expect(result.current.themeId).toBe('default');
    expect(result.current.themes.map((t) => t.theme.id)).not.toContain('night');
    expect(result.current.face).toBe('light');
  });

  it('keeps the theme on screen when an import is refused', async () => {
    stubOsDark(true);
    const { result } = renderHook(() => useThemeProvider());
    await expect(result.current.importPack('{')).rejects.toThrow('the file is not JSON');
    expect(result.current.themeId).toBe('default');
  });
});
