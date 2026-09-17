import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { agentAccentTriplet } from '../../lib/agentIdentity';
import {
  applyTheme,
  CSS_CACHE_KEY,
  FACE_CACHE_KEY,
  resolveFace,
  STYLE_ELEMENT_ID,
  THEME_APPLIED_EVENT,
  themeCss,
} from '../apply';
import { type Theme, validateThemePack } from '../validate';

function bundled(id: string): Theme {
  const result = validateThemePack(readFileSync(`src/themes/packs/${id}.json`, 'utf8'));
  if (!result.ok) throw new Error(result.errors.join('; '));
  return result.theme;
}

const DARK_ONLY = JSON.stringify({
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

function nightTheme(): Theme {
  const result = validateThemePack(DARK_ONLY);
  if (!result.ok) throw new Error(result.errors.join('; '));
  return result.theme;
}

/** `--token: value` pairs of one rule of a generated stylesheet. */
function rule(css: string, selector: string): Record<string, string> {
  const line = css.split('\n').find((l) => l.startsWith(`${selector} {`));
  expect(line, `${selector} rule`).toBeDefined();
  return Object.fromEntries([...(line ?? '').matchAll(/--([\w-]+): ([^;]+);/g)].map((m) => [m[1], m[2]]));
}

afterEach(() => {
  localStorage.clear();
  document.documentElement.className = '';
  document.documentElement.removeAttribute('style');
  delete document.documentElement.dataset.accent;
  document.getElementById(STYLE_ELEMENT_ID)?.remove();
  applyTheme(bundled('default'), 'dark', true);
  localStorage.clear();
  document.getElementById(STYLE_ELEMENT_ID)?.remove();
  document.documentElement.className = '';
});

describe('the stylesheet of a theme', () => {
  it('writes light under :root and dark under .dark, each outranking the bundled stylesheet', () => {
    const css = themeCss(bundled('default'));
    expect(rule(css, ':root:root')['surface-base']).toBe('190 6% 92%');
    expect(rule(css, ':root.dark')['surface-base']).toBe('190 6% 8%');
    expect(css.indexOf(':root:root')).toBeLessThan(css.indexOf(':root.dark'));
  });

  it('writes the overlay as a whole colour, since it is used as one', () => {
    expect(rule(themeCss(bundled('default')), ':root.dark')['surface-overlay']).toBe('hsl(0 0% 0% / 0.7)');
  });

  it('writes a role colour only when the pack gives one', () => {
    expect('surface-panel' in rule(themeCss(bundled('default')), ':root.dark')).toBe(false);
    expect(rule(themeCss(bundled('legacy')), ':root.dark')['surface-panel']).toBe('222.22 47.37% 11.18%');
  });

  it('writes a fixed accent into the stylesheet, and no accent when it is the agent', () => {
    expect(rule(themeCss(bundled('legacy')), ':root:root').agent).toBe('229.89 78.63% 54.12%');
    expect(rule(themeCss(bundled('legacy')), ':root:root')['agent-ink']).toBe('0 0% 100%');
    expect('agent' in rule(themeCss(bundled('default')), ':root:root')).toBe(false);
  });

  it('writes a one-face theme for both selectors', () => {
    const css = themeCss(nightTheme());
    expect(css.startsWith(':root:root,:root.dark {')).toBe(true);
    expect(css).toContain('--surface-base: 260 20% 6%;');
  });

  it('is built from the parsed numbers, not from the pack text', () => {
    const spaced = DARK_ONLY.replace('"260 20% 6%"', '"  260   20%   6%  "');
    const result = validateThemePack(spaced);
    expect(result.ok && themeCss(result.theme)).toContain('--surface-base: 260 20% 6%;');
  });
});

describe('the face that is drawn', () => {
  it('follows the mode, and the OS under system', () => {
    const theme = bundled('default');
    expect(resolveFace(theme, 'light', true)).toBe('light');
    expect(resolveFace(theme, 'dark', false)).toBe('dark');
    expect(resolveFace(theme, 'system', true)).toBe('dark');
    expect(resolveFace(theme, 'system', false)).toBe('light');
  });

  it('is the only face of a one-face theme, whatever the mode says', () => {
    expect(resolveFace(nightTheme(), 'light', false)).toBe('dark');
  });
});

describe('applying a theme', () => {
  it('replaces the one style element, sets the dark class, and caches what index.html re-inserts', () => {
    applyTheme(bundled('legacy'), 'light', true);
    applyTheme(bundled('default'), 'dark', false);
    const styles = document.querySelectorAll(`#${STYLE_ELEMENT_ID}`);
    expect(styles).toHaveLength(1);
    expect(styles[0].textContent).toBe(themeCss(bundled('default')));
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(localStorage.getItem(CSS_CACHE_KEY)).toBe(themeCss(bundled('default')));
    expect(localStorage.getItem(FACE_CACHE_KEY)).toBe('');
  });

  it('caches the forced face of a one-face theme', () => {
    expect(applyTheme(nightTheme(), 'light', false)).toBe('dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(localStorage.getItem(FACE_CACHE_KEY)).toBe('dark');
  });

  it('flags a fixed accent, clears the inline one, and unflags on the way back', () => {
    const root = document.documentElement;
    root.style.setProperty('--agent', '10 70% 60%');
    root.style.setProperty('--agent-ink', '0 0% 100%');
    applyTheme(bundled('legacy'), 'dark', true);
    expect(root.dataset.accent).toBe('fixed');
    expect(root.style.getPropertyValue('--agent')).toBe('');
    expect(root.style.getPropertyValue('--agent-ink')).toBe('');
    applyTheme(bundled('default'), 'dark', true);
    expect(root.dataset.accent).toBeUndefined();
  });

  it('says it has applied, so an accent written earlier can be written again', () => {
    const heard = vi.fn();
    window.addEventListener(THEME_APPLIED_EVENT, heard);
    applyTheme(bundled('default'), 'dark', true);
    window.removeEventListener(THEME_APPLIED_EVENT, heard);
    expect(heard).toHaveBeenCalledTimes(1);
  });

  it('corrects an agent accent against the raised surface of the face on screen', () => {
    // Hue 240 (blue) is the hard case: it has to move a long way from 58%.
    applyTheme(bundled('default'), 'dark', true);
    const onDark = Number(/ (\d+)%$/.exec(agentAccentTriplet({ id: 'x', metadata: { accent: '240 70% 58%' } }))?.[1]);
    applyTheme(bundled('default'), 'light', false);
    const onLight = Number(/ (\d+)%$/.exec(agentAccentTriplet({ id: 'x', metadata: { accent: '240 70% 58%' } }))?.[1]);
    expect(onDark).toBeGreaterThan(58);
    expect(onLight).toBeLessThan(58);
  });
});

// ---------------------------------------------------------------------------
// The default pack and the stylesheet's fallback colours are the same palette
// written twice (the fallback is what a page draws before its script runs).
describe('the fallback colours in index.css', () => {
  const css = readFileSync('src/index.css', 'utf8');

  function fallback(selector: string): Record<string, string> {
    const start = css.indexOf(`\n  ${selector} {`);
    expect(start, `${selector} block`).toBeGreaterThan(-1);
    const body = css.slice(start, css.indexOf('\n  }', start));
    const hue = /--h: (\d+);/.exec(css)?.[1] ?? '';
    const out: Record<string, string> = {};
    for (const m of body.matchAll(/--([\w-]+): var\(--h\) ([\d.]+% [\d.]+%);/g)) out[m[1]] = `${hue} ${m[2]}`;
    const overlay = /--surface-overlay: rgba\(0, 0, 0, ([\d.]+)\);/.exec(body);
    if (overlay) out['surface-overlay'] = `hsl(0 0% 0% / ${overlay[1]})`;
    return out;
  }

  it('are the default pack, light', () => {
    const generated = rule(themeCss(bundled('default')), ':root:root');
    const inStylesheet = fallback(':root');
    delete inStylesheet.agent;
    delete inStylesheet['agent-ink'];
    expect(inStylesheet).toEqual(generated);
  });

  it('are the default pack, dark', () => {
    expect(fallback('.dark')).toEqual(rule(themeCss(bundled('default')), ':root.dark'));
  });

  it('no longer carry a second palette', () => {
    expect(css).not.toMatch(/--agent:[^;]*!important/);
    expect([...css.matchAll(/--surface-base:/g)]).toHaveLength(2);
  });
});

// ---------------------------------------------------------------------------
// The Legacy pack is "the colours the dashboard had": pinned to the values the
// stylesheet carried before the redesign (RGB), so the HSL spelling in the pack
// cannot drift from them unnoticed.
type Rgb = [number, number, number];

function hslToRgb255(h: number, s: number, l: number): Rgb {
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  const [r, g, b] =
    h < 60
      ? [c, x, 0]
      : h < 120
        ? [x, c, 0]
        : h < 180
          ? [0, c, x]
          : h < 240
            ? [0, x, c]
            : h < 300
              ? [x, 0, c]
              : [c, 0, x];
  return [r, g, b].map((v) => Math.round((v + m) * 255)) as Rgb;
}

/** A translucent colour composited over an opaque one, as the old glass surfaces were. */
function over(fg: Rgb, alpha: number, bg: Rgb): Rgb {
  return fg.map((v, i) => Math.round(v * alpha + bg[i] * (1 - alpha))) as Rgb;
}

const OLD_LIGHT = { base: [248, 250, 252] as Rgb, primary: [255, 255, 255] as Rgb, secondary: [241, 245, 249] as Rgb };
const OLD_DARK = { base: [15, 23, 42] as Rgb, primary: [30, 41, 59] as Rgb, secondary: [30, 41, 59] as Rgb };

const EXPECTED: Record<'light' | 'dark', Record<string, Rgb>> = {
  light: {
    'surface-base': OLD_LIGHT.base,
    'surface-secondary': OLD_LIGHT.secondary,
    'surface-primary': OLD_LIGHT.primary,
    'border-default': [226, 232, 240],
    'border-subtle': [241, 245, 249],
    'text-primary': [30, 41, 59],
    'text-secondary': [100, 116, 139],
    'text-tertiary': [148, 163, 184],
    'text-muted': [203, 213, 225],
    // bg-glass / -strong / -subtle were white at 40 / 60 / 80 %; card-solid was primary at 50 %.
    'surface-panel': over([255, 255, 255], 0.4, OLD_LIGHT.base),
    'surface-field': over([255, 255, 255], 0.6, OLD_LIGHT.base),
    'surface-control': over([255, 255, 255], 0.8, OLD_LIGHT.base),
    'surface-card': over(OLD_LIGHT.primary, 0.5, OLD_LIGHT.base),
    agent: [46, 77, 230],
    'agent-ink': [255, 255, 255],
  },
  dark: {
    'surface-base': OLD_DARK.base,
    'surface-secondary': OLD_DARK.secondary,
    'surface-primary': OLD_DARK.primary,
    'border-default': [51, 65, 85],
    'border-subtle': [30, 41, 59],
    'text-primary': [241, 245, 249],
    'text-secondary': [203, 213, 225],
    'text-tertiary': [148, 163, 184],
    'text-muted': [71, 85, 105],
    // bg-glass / -strong were slate-900 at 60 / 80 %, -subtle slate-800 at 80 %.
    'surface-panel': over([15, 23, 42], 0.6, OLD_DARK.base),
    'surface-field': over([15, 23, 42], 0.8, OLD_DARK.base),
    'surface-control': over([30, 41, 59], 0.8, OLD_DARK.base),
    'surface-card': over(OLD_DARK.primary, 0.5, OLD_DARK.base),
    agent: [91, 122, 255],
    'agent-ink': [255, 255, 255],
  },
};

function drawn(selector: string): Record<string, Rgb> {
  const out: Record<string, Rgb> = {};
  for (const [token, value] of Object.entries(rule(themeCss(bundled('legacy')), selector))) {
    const m = /^([\d.]+) ([\d.]+)% ([\d.]+)%$/.exec(value);
    if (m) out[token] = hslToRgb255(Number(m[1]), Number(m[2]) / 100, Number(m[3]) / 100);
  }
  return out;
}

describe('the legacy palette', () => {
  it('draws every light token in the colour it had before the redesign', () => {
    expect(drawn(':root:root')).toEqual(EXPECTED.light);
  });

  it('draws every dark token in the colour it had before the redesign', () => {
    expect(drawn(':root.dark')).toEqual(EXPECTED.dark);
  });
});

// ---------------------------------------------------------------------------
// index.html repeats, before React, what the hook and applyTheme decide after
// it. The script is run here as it is written there.
describe('the script in index.html that runs before React', () => {
  const html = readFileSync('index.html', 'utf8');
  const script = /<script>\s*([\s\S]*?)<\/script>/.exec(html)?.[1] ?? '';

  function boot(stored: Record<string, string>, osDark: boolean) {
    for (const [k, v] of Object.entries(stored)) localStorage.setItem(k, v);
    vi.stubGlobal('matchMedia', () => ({ matches: osDark }));
    new Function(script)();
    vi.unstubAllGlobals();
    return document.documentElement.classList.contains('dark');
  }

  it('is there, and names no theme', () => {
    expect(script).toContain(CSS_CACHE_KEY);
    expect(script).toContain(FACE_CACHE_KEY);
    expect(script).toContain(`'${STYLE_ELEMENT_ID}'`);
  });

  it('is dark unless a mode says otherwise, and follows the OS under system', () => {
    expect(boot({}, false)).toBe(true);
    document.documentElement.className = '';
    expect(boot({ 'cloto-theme-mode': 'light' }, true)).toBe(false);
    expect(boot({ 'cloto-theme-mode': 'system' }, true)).toBe(true);
    document.documentElement.className = '';
    expect(boot({ 'cloto-theme-mode': 'system' }, false)).toBe(false);
  });

  it('reads the old single setting until the app has split it', () => {
    expect(boot({ 'cloto-theme': 'light' }, true)).toBe(false);
    localStorage.clear();
    // A theme name stored there followed the OS.
    expect(boot({ 'cloto-theme': 'some-theme' }, true)).toBe(true);
  });

  it('draws the face a one-face theme forces, whatever the mode says', () => {
    expect(boot({ 'cloto-theme-mode': 'light', [FACE_CACHE_KEY]: 'dark' }, false)).toBe(true);
    document.documentElement.className = '';
    localStorage.clear();
    expect(boot({ 'cloto-theme-mode': 'dark', [FACE_CACHE_KEY]: 'light' }, true)).toBe(false);
  });

  it('re-inserts the cached stylesheet in the element the app then reuses', () => {
    const css = themeCss(bundled('legacy'));
    boot({ [CSS_CACHE_KEY]: css }, true);
    expect(document.getElementById(STYLE_ELEMENT_ID)?.textContent).toBe(css);
    applyTheme(bundled('default'), 'dark', true);
    expect(document.querySelectorAll(`#${STYLE_ELEMENT_ID}`)).toHaveLength(1);
    expect(document.getElementById(STYLE_ELEMENT_ID)?.textContent).toBe(themeCss(bundled('default')));
  });
});
