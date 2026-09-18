import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const tauri = vi.hoisted(() => ({
  scanThemesDir: vi.fn(async (): Promise<Array<[string, string]>> => []),
  saveThemePack: vi.fn(async (): Promise<boolean> => false),
  removeThemePack: vi.fn(async (): Promise<boolean> => false),
}));
vi.mock('../../lib/tauri', async (original) => ({ ...(await original<typeof import('../../lib/tauri')>()), ...tauri }));

import {
  BROWSER_PACKS_KEY,
  exportThemeTemplate,
  findTheme,
  getRejectedPacks,
  getThemes,
  importThemePack,
  loadThemes,
  removeThemePack,
} from '../load';
import { validateThemePack } from '../validate';

function pack(id: string, label: string, extra: Record<string, unknown> = {}): string {
  return JSON.stringify({
    schema: 1,
    id,
    label,
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
    ...extra,
  });
}

const ids = () => getThemes().map((t) => t.theme.id);

beforeEach(async () => {
  localStorage.clear();
  tauri.scanThemesDir.mockResolvedValue([]);
  tauri.saveThemePack.mockResolvedValue(false);
  await loadThemes();
});

afterEach(() => {
  vi.clearAllMocks();
});

describe('the themes on offer', () => {
  it('are the bundled ones, the fallback first, with nothing external', () => {
    expect(ids()).toEqual(['default', 'legacy']);
    expect(getThemes().every((t) => t.source === 'bundled')).toBe(true);
  });

  it('include a pack that was placed in the themes directory', async () => {
    tauri.scanThemesDir.mockResolvedValue([['night', pack('night', 'Night')]]);
    await loadThemes();
    expect(ids()).toEqual(['default', 'legacy', 'night']);
    expect(findTheme('night').source).toBe('directory');
  });

  it('leave out a pack that does not validate, and keep why', async () => {
    tauri.scanThemesDir.mockResolvedValue([
      ['broken', '{'],
      ['night', pack('night', 'Night')],
    ]);
    await loadThemes();
    expect(ids()).toEqual(['default', 'legacy', 'night']);
    expect(getRejectedPacks()).toEqual([{ name: 'broken', errors: ['the file is not JSON'] }]);
  });

  it('leave out an external pack that takes a bundled id', async () => {
    tauri.scanThemesDir.mockResolvedValue([['mine', pack('legacy', 'Mine')]]);
    await loadThemes();
    expect(ids()).toEqual(['default', 'legacy']);
    expect(findTheme('legacy').theme.label).toBe('Legacy');
    expect(getRejectedPacks()).toEqual([{ name: 'mine', errors: ['the id "legacy" is already taken'] }]);
  });

  it('draw the fallback for an id that is gone', () => {
    expect(findTheme('removed-long-ago').theme.id).toBe('default');
    expect(findTheme(null).theme.id).toBe('default');
  });
});

describe('importing a pack', () => {
  it('in a browser keeps it in storage, so it is still there after a reload', async () => {
    const loaded = await importThemePack(pack('night', 'Night'));
    expect(loaded.source).toBe('browser');
    expect(ids()).toContain('night');
    // A reload: module state is rebuilt from the sources.
    await loadThemes();
    expect(ids()).toEqual(['default', 'legacy', 'night']);
    expect(Object.keys(JSON.parse(localStorage.getItem(BROWSER_PACKS_KEY) ?? '{}'))).toEqual(['night']);
  });

  it('on the desktop saves it to the themes directory under its id, and not to storage', async () => {
    tauri.saveThemePack.mockResolvedValue(true);
    const json = pack('night', 'Night');
    const loaded = await importThemePack(json);
    expect(tauri.saveThemePack).toHaveBeenCalledWith('night', json);
    expect(loaded.source).toBe('directory');
    expect(localStorage.getItem(BROWSER_PACKS_KEY)).toBeNull();
  });

  it('refuses one that does not validate, with the reasons, and keeps nothing', async () => {
    await expect(importThemePack(pack('night', 'Night', { css: 'x' }))).rejects.toThrow('unknown key "css"');
    expect(ids()).toEqual(['default', 'legacy']);
    expect(tauri.saveThemePack).not.toHaveBeenCalled();
    expect(localStorage.getItem(BROWSER_PACKS_KEY)).toBeNull();
  });

  it('refuses one that takes a bundled id', async () => {
    await expect(importThemePack(pack('default', 'Mine'))).rejects.toThrow('belongs to a built-in theme');
    expect(findTheme('default').theme.label).toBe('Cloto');
  });

  it('replaces an earlier import of the same id instead of listing it twice', async () => {
    await importThemePack(pack('night', 'Night'));
    await importThemePack(pack('night', 'Night II'));
    expect(ids()).toEqual(['default', 'legacy', 'night']);
    expect(findTheme('night').theme.label).toBe('Night II');
  });
});

describe('removing a pack', () => {
  it('forgets an imported one, in the directory and in storage', async () => {
    await importThemePack(pack('night', 'Night'));
    await removeThemePack('night');
    expect(ids()).toEqual(['default', 'legacy']);
    expect(tauri.removeThemePack).toHaveBeenCalledWith('night');
    await loadThemes();
    expect(ids()).toEqual(['default', 'legacy']);
  });

  it('does not remove a bundled one', async () => {
    await removeThemePack('legacy');
    expect(ids()).toEqual(['default', 'legacy']);
    expect(tauri.removeThemePack).not.toHaveBeenCalled();
  });
});

describe('the template', () => {
  it('is a pack that validates once it is given an id of its own', () => {
    const result = validateThemePack(exportThemeTemplate());
    expect(result.ok && result.theme.id).toBe('my-theme');
    expect(result.ok && Object.keys(result.theme.faces).sort()).toEqual(['dark', 'light']);
  });
});

// ---------------------------------------------------------------------------
// A theme is data: the code knows the fallback's id and no other. The ids are
// read from the packs directory, so a pack added later is covered without
// touching this test.
describe('theme ids outside the packs', () => {
  function sources(dir: string): string[] {
    return readdirSync(dir).flatMap((name) => {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) {
        return name === '__tests__' || path === join('src', 'themes', 'packs') || name === 'locales'
          ? []
          : sources(path);
      }
      return /\.(ts|tsx|css)$/.test(name) && name !== 'compiled-tailwind.css' ? [path] : [];
    });
  }

  it('are not written into a component, a hook, a stylesheet or index.html', () => {
    const named = readdirSync('src/themes/packs')
      .map((f) => f.replace(/\.json$/, ''))
      .filter((id) => id !== 'default');
    expect(named.length).toBeGreaterThan(0);
    const files = [...sources('src'), 'index.html'];
    expect(files.length).toBeGreaterThan(50);
    // Everywhere: the class and key spellings a theme's name used to have.
    // In the files that used to list themes: the bare id as a string too — a
    // word like "legacy" is an ordinary value elsewhere in the dashboard.
    const LISTED_THEMES = [
      'index.html',
      'src/main.tsx',
      'src/index.css',
      'src/hooks/useTheme.ts',
      'src/lib/agentIdentity.tsx',
      'src/contexts/AgentContext.tsx',
      'src/components/SetupWizard.tsx',
      'src/components/ThemeProvider.tsx',
      'src/components/settings/GeneralSection.tsx',
      'src/components/settings/ThemeSettings.tsx',
      'src/themes/apply.ts',
      'src/themes/load.ts',
      'src/themes/validate.ts',
    ];
    for (const listed of LISTED_THEMES) expect(files, listed).toContain(listed);
    const hits: string[] = [];
    for (const file of files) {
      const text = readFileSync(file, 'utf8');
      for (const id of named) {
        const spelled = new RegExp(`theme[-_]${id}`).test(text);
        const quoted = LISTED_THEMES.includes(file) && new RegExp(`['"\`]${id}['"\`]`).test(text);
        if (spelled || quoted) hits.push(`${file}: ${id}`);
      }
    }
    expect(hits).toEqual([]);
  });
});
