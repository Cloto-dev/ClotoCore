/**
 * Where themes come from (docs/THEME_PACKS_DESIGN.md §2 (g)):
 *
 *   1. bundled — in the bundle, so a browser session has them too;
 *   2. the themes directory — desktop only, scanned through Tauri;
 *   3. imported — kept in that directory on the desktop and in `localStorage`
 *      in a browser, which is what keeps external themes from being desktop-only.
 *
 * Every source goes through `validateThemePack`. A pack that does not pass is
 * not offered; it is kept with its reasons so the settings page can say why.
 */
import { removeThemePack as removeFile, saveThemePack as saveFile, scanThemesDir } from '../lib/tauri';
import { type Theme, validateThemePack } from './validate';

export type ThemeSource = 'bundled' | 'directory' | 'browser';

export interface LoadedTheme {
  theme: Theme;
  warnings: string[];
  source: ThemeSource;
}

export interface RejectedPack {
  name: string;
  errors: string[];
}

/** The theme a fresh install draws, and the one to fall back to. It is the
 * bundled pack the stylesheet's own fallback colours mirror. */
// HARDCODED(dashboard/src/themes/packs/default.json::id): the one theme id the code knows — what to draw when the stored id is gone.
export const FALLBACK_THEME_ID = 'default';
export const BROWSER_PACKS_KEY = 'cloto-theme-packs';

const bundledSources = import.meta.glob('./packs/*.json', {
  eager: true,
  query: '?raw',
  import: 'default',
}) as Record<string, string>;

function readBundled(): LoadedTheme[] {
  const out: LoadedTheme[] = [];
  for (const [path, json] of Object.entries(bundledSources)) {
    const result = validateThemePack(json);
    // A bundled pack that does not validate is a build defect; the test over
    // the bundled packs is what catches it, not a user's console.
    if (!result.ok) throw new Error(`bundled theme ${path} is invalid: ${result.errors.join('; ')}`);
    out.push({ theme: result.theme, warnings: result.warnings, source: 'bundled' });
  }
  // The fallback first, the rest by label: the order the pickers show.
  return out.sort((a, b) =>
    a.theme.id === FALLBACK_THEME_ID
      ? -1
      : b.theme.id === FALLBACK_THEME_ID
        ? 1
        : a.theme.label.localeCompare(b.theme.label),
  );
}

const bundled = readBundled();
let external: LoadedTheme[] = [];
let rejected: RejectedPack[] = [];

function readBrowserPacks(): Record<string, string> {
  try {
    const raw = JSON.parse(localStorage.getItem(BROWSER_PACKS_KEY) ?? '{}');
    if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return {};
    return Object.fromEntries(Object.entries(raw).filter((e): e is [string, string] => typeof e[1] === 'string'));
  } catch {
    return {};
  }
}

function writeBrowserPacks(packs: Record<string, string>) {
  localStorage.setItem(BROWSER_PACKS_KEY, JSON.stringify(packs));
}

function isBundledId(id: string): boolean {
  return bundled.some((b) => b.theme.id === id);
}

/** Read the external packs. Call (and await) before React renders. */
export async function loadThemes(): Promise<void> {
  const found: LoadedTheme[] = [];
  const refused: RejectedPack[] = [];
  const take = (name: string, json: string, source: ThemeSource) => {
    const result = validateThemePack(json);
    if (!result.ok) {
      refused.push({ name, errors: result.errors });
    } else if (isBundledId(result.theme.id) || found.some((f) => f.theme.id === result.theme.id)) {
      refused.push({ name, errors: [`the id "${result.theme.id}" is already taken`] });
    } else {
      found.push({ theme: result.theme, warnings: result.warnings, source });
    }
  };
  for (const [name, json] of await scanThemesDir()) take(name, json, 'directory');
  for (const [name, json] of Object.entries(readBrowserPacks())) take(name, json, 'browser');
  external = found.sort((a, b) => a.theme.label.localeCompare(b.theme.label));
  rejected = refused;
}

export function getThemes(): LoadedTheme[] {
  return [...bundled, ...external];
}

export function getRejectedPacks(): RejectedPack[] {
  return rejected;
}

export function findTheme(id: string | null): LoadedTheme {
  return getThemes().find((t) => t.theme.id === id) ?? bundled[0];
}

/** Validate a pack and keep it. Throws with the reasons when it is refused. */
export async function importThemePack(json: string): Promise<LoadedTheme> {
  const result = validateThemePack(json);
  if (!result.ok) throw new Error(result.errors.join('; '));
  const { id } = result.theme;
  if (isBundledId(id)) throw new Error(`the id "${id}" belongs to a built-in theme`);
  const savedToDisk = await saveFile(id, json);
  if (!savedToDisk) writeBrowserPacks({ ...readBrowserPacks(), [id]: json });
  const loaded: LoadedTheme = {
    theme: result.theme,
    warnings: result.warnings,
    source: savedToDisk ? 'directory' : 'browser',
  };
  external = [...external.filter((t) => t.theme.id !== id), loaded].sort((a, b) =>
    a.theme.label.localeCompare(b.theme.label),
  );
  return loaded;
}

/** Forget an external pack. Bundled ones cannot be removed. */
export async function removeThemePack(id: string): Promise<void> {
  if (isBundledId(id)) return;
  await removeFile(id);
  const packs = readBrowserPacks();
  if (id in packs) {
    delete packs[id];
    writeBrowserPacks(packs);
  }
  external = external.filter((t) => t.theme.id !== id);
}

/** The fallback pack as a starting point for a new one. */
export function exportThemeTemplate(): string {
  const source = Object.entries(bundledSources).find(([path]) => path.endsWith(`/${FALLBACK_THEME_ID}.json`));
  const pack = JSON.parse(source ? source[1] : '{}');
  return JSON.stringify({ ...pack, id: 'my-theme', label: 'My theme', author: '', license: '' }, null, 2);
}
