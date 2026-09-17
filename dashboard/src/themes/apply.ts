/**
 * Put a validated theme on the page (docs/THEME_PACKS_DESIGN.md §2 (c), (d)).
 *
 * The stylesheet is written from the numbers `validate.ts` parsed, never from a
 * pack's own strings. It lives in one `<style>` element that is replaced
 * wholesale, so nothing of the previous theme stays behind.
 */
import { setAccentSurface } from '../lib/agentIdentity';
import { FACES, type FaceName, type Hsl, hslToRgb, OPTIONAL_TOKENS, REQUIRED_TOKENS, type Theme } from './validate';

export type ThemeMode = 'light' | 'dark' | 'system';

export const STYLE_ELEMENT_ID = 'cloto-theme';
/** The generated stylesheet of the last apply; `index.html` re-inserts it before React. */
export const CSS_CACHE_KEY = 'cloto-theme-css';
/** The face a one-face theme forces, or empty; `index.html` reads it to set `dark`. */
export const FACE_CACHE_KEY = 'cloto-theme-face';
/** Sent on `window` after every apply. The accent is computed against the
 * surface on screen, so whoever wrote one has to write it again. */
export const THEME_APPLIED_EVENT = 'cloto-theme-applied';

// The selectors outrank `:root` / `.dark` in the bundled stylesheet whatever the
// order the two end up in: before React, the cached copy is inserted ahead of
// the bundle's <link>. Light first, so dark wins where both match.
const SELECTOR: Record<FaceName, string> = { light: ':root:root', dark: ':root.dark' };

function triplet([h, s, l]: Hsl): string {
  return `${h} ${s}% ${l}%`;
}

function faceCss(theme: Theme, name: FaceName): string {
  const face = theme.faces[name];
  if (!face) return '';
  const lines: string[] = [];
  for (const token of [...REQUIRED_TOKENS, ...OPTIONAL_TOKENS]) {
    const value = face.colours[token];
    if (value) lines.push(`--${token}: ${triplet(value)};`);
  }
  const [h, s, l, a] = face.overlay;
  lines.push(`--surface-overlay: hsl(${h} ${s}% ${l}% / ${a});`);
  const fixed = theme.accent?.[name];
  if (fixed) {
    lines.push(`--agent: ${triplet(fixed.agent)};`);
    lines.push(`--agent-ink: ${triplet(fixed.ink)};`);
  }
  return `${SELECTOR[name]} { ${lines.join(' ')} }`;
}

/** The stylesheet of a theme. A one-face theme is written for both selectors,
 * so it looks the same whichever way the `dark` class happens to be set. */
export function themeCss(theme: Theme): string {
  const present = FACES.filter((f) => theme.faces[f]);
  if (present.length === 1) {
    const only = present[0];
    const body = faceCss(theme, only).slice(SELECTOR[only].length);
    return `${SELECTOR.light},${SELECTOR.dark}${body}`;
  }
  return present.map((f) => faceCss(theme, f)).join('\n');
}

/** The face that is drawn: the theme's only one, or the one the mode asks for. */
export function resolveFace(theme: Theme, mode: ThemeMode, osDark: boolean): FaceName {
  const present = FACES.filter((f) => theme.faces[f]);
  if (present.length === 1) return present[0];
  if (mode === 'system') return osDark ? 'dark' : 'light';
  return mode;
}

function store(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Storage full or unavailable: the theme still applies, it just flashes the
    // default palette on the next load.
  }
}

/** Apply `theme` in `mode`; returns the face that ended up on screen. */
export function applyTheme(
  theme: Theme,
  mode: ThemeMode,
  osDark: boolean,
  doc: Document = document,
  win: Window = window,
): FaceName {
  const face = resolveFace(theme, mode, osDark);
  const root = doc.documentElement;
  const css = themeCss(theme);

  let style = doc.getElementById(STYLE_ELEMENT_ID);
  if (!style) {
    style = doc.createElement('style');
    style.id = STYLE_ELEMENT_ID;
    doc.head.appendChild(style);
  }
  style.textContent = css;
  root.classList.toggle('dark', face === 'dark');

  if (theme.accent) {
    root.dataset.accent = 'fixed';
    // An inline `--agent` from the present agent would outrank the stylesheet.
    root.style.removeProperty('--agent');
    root.style.removeProperty('--agent-ink');
  } else {
    delete root.dataset.accent;
  }
  const drawn = theme.faces[face];
  if (drawn) setAccentSurface(hslToRgb(drawn.colours['surface-primary']));

  store(CSS_CACHE_KEY, css);
  store(FACE_CACHE_KEY, FACES.filter((f) => theme.faces[f]).length === 1 ? face : '');
  win.dispatchEvent(new Event(THEME_APPLIED_EVENT));
  return face;
}
