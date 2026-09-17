import { readFileSync } from 'node:fs';
import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { agentColor } from '../../lib/agentIdentity';
import { useThemeProvider } from '../useTheme';

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

afterEach(() => {
  localStorage.clear();
  document.documentElement.className = '';
  vi.unstubAllGlobals();
});

describe('theme preference', () => {
  it('is dark when nothing was chosen, whatever the OS says', () => {
    stubOsDark(false);
    const { result } = renderHook(() => useThemeProvider());
    expect(result.current.preference).toBe('dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(document.documentElement.classList.contains('theme-legacy')).toBe(false);
  });

  it('puts the legacy palette on, follows the OS for light and dark, and takes it off again', () => {
    stubOsDark(false);
    const { result } = renderHook(() => useThemeProvider());

    act(() => result.current.setPreference('legacy'));
    const root = document.documentElement.classList;
    expect(root.contains('theme-legacy')).toBe(true);
    expect(root.contains('dark')).toBe(false);
    expect(localStorage.getItem('cloto-theme')).toBe('legacy');

    act(() => result.current.setPreference('dark'));
    expect(root.contains('theme-legacy')).toBe(false);
    expect(root.contains('dark')).toBe(true);
  });

  it('restores legacy in dark from storage when the OS is dark', () => {
    stubOsDark(true);
    localStorage.setItem('cloto-theme', 'legacy');
    renderHook(() => useThemeProvider());
    expect(document.documentElement.classList.contains('theme-legacy')).toBe(true);
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });

  it('gives every agent the stylesheet accent under legacy, and its own hue otherwise', () => {
    const root = document.createElement('html');
    expect(agentColor({ id: 'agent.a' }, root)).toMatch(/^hsl\(\d+ 70% \d+%\)$/);
    root.classList.add('theme-legacy');
    expect(agentColor({ id: 'agent.a' }, root)).toBe('hsl(var(--agent))');
  });
});

// ---------------------------------------------------------------------------
// The legacy palette is "the colours the dashboard had": pinned here to the
// values the stylesheet carried before the redesign (RGB triplets), so the
// HSL spelling in index.css cannot drift from them unnoticed.

type Rgb = [number, number, number];

function hslToRgb(h: number, s: number, l: number): Rgb {
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

/** A translucent colour composited over an opaque one, as the old glass
 * surfaces were over the base. */
function over(fg: Rgb, alpha: number, bg: Rgb): Rgb {
  return fg.map((v, i) => Math.round(v * alpha + bg[i] * (1 - alpha))) as Rgb;
}

const OLD_LIGHT = {
  base: [248, 250, 252] as Rgb,
  primary: [255, 255, 255] as Rgb,
  secondary: [241, 245, 249] as Rgb,
};
const OLD_DARK = {
  base: [15, 23, 42] as Rgb,
  primary: [30, 41, 59] as Rgb,
  secondary: [30, 41, 59] as Rgb,
};

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

function block(css: string, selector: string): Record<string, Rgb> {
  const start = css.indexOf(`${selector} {`);
  expect(start, `${selector} block`).toBeGreaterThan(-1);
  const body = css.slice(start, css.indexOf('\n  }', start));
  const tokens: Record<string, Rgb> = {};
  for (const m of body.matchAll(/--([\w-]+): ([\d.]+) ([\d.]+)% ([\d.]+)%/g)) {
    tokens[m[1]] = hslToRgb(Number(m[2]), Number(m[3]) / 100, Number(m[4]) / 100);
  }
  return tokens;
}

describe('the legacy palette', () => {
  // Read from disk (relative to dashboard/, where vitest runs): a CSS import
  // resolves to an empty module under vitest.
  const css = readFileSync('src/index.css', 'utf8');

  // Two plain cases rather than `it.each`: the docs gate counts test cases
  // statically, and a table it cannot expand makes the count disagree with
  // the run.
  it('draws every light token in the colour it had before the redesign', () => {
    expect(block(css, '.theme-legacy')).toEqual(EXPECTED.light);
  });

  it('draws every dark token in the colour it had before the redesign', () => {
    expect(block(css, '.theme-legacy.dark')).toEqual(EXPECTED.dark);
  });

  it('holds the accent against the per-agent value written inline', () => {
    // AgentProvider writes --agent on <html> as an inline style, which beats a
    // class rule unless the rule is important.
    for (const selector of ['.theme-legacy', '.theme-legacy.dark']) {
      const start = css.indexOf(`${selector} {`);
      const body = css.slice(start, css.indexOf('\n  }', start));
      expect(body).toMatch(/--agent: [\d.]+ [\d.]+% [\d.]+% !important;/);
    }
  });
});
