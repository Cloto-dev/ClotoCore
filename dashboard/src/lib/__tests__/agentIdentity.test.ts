import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { accentLightness, agentAccentTriplet, agentHue, applyPresentAgent } from '../agentIdentity';

// An HSL→sRGB conversion written differently from the one under test (the
// hue-to-channel form), so a mistake in the module's version cannot agree with
// itself here.
function hueToChannel(p: number, q: number, t: number): number {
  let x = t;
  if (x < 0) x += 1;
  if (x > 1) x -= 1;
  if (x < 1 / 6) return p + (q - p) * 6 * x;
  if (x < 1 / 2) return q;
  if (x < 2 / 3) return p + (q - p) * (2 / 3 - x) * 6;
  return p;
}

function hsl(h: number, s: number, l: number): [number, number, number] {
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const t = h / 360;
  return [hueToChannel(p, q, t + 1 / 3), hueToChannel(p, q, t), hueToChannel(p, q, t - 1 / 3)];
}

function contrast(a: [number, number, number], b: [number, number, number]): number {
  const lum = (c: [number, number, number]) =>
    c
      .map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4))
      .reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i], 0);
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

describe('agent accent', () => {
  it('keeps an agent on one hue, and gives different agents different hues', () => {
    expect(agentHue({ id: 'agent.sapphy' })).toBe(agentHue({ id: 'agent.sapphy' }));
    const hues = ['agent.sapphy', 'agent.ks22', 'agent.cloto_default', 'agent.a', 'agent.b'].map((id) =>
      agentHue({ id }),
    );
    for (const h of hues) {
      expect(Number.isInteger(h)).toBe(true);
      expect(h).toBeGreaterThanOrEqual(0);
      expect(h).toBeLessThan(360);
    }
    expect(new Set(hues).size).toBe(hues.length);
  });

  it('leaves the lightness at 58% where that already reads, and raises it where it does not', () => {
    // Cyan reads at 58% (7:1 on the raised surface); blue at 58% is 2.4:1.
    expect(accentLightness(190)).toBe(58);
    expect(accentLightness(240)).toBe(72);
  });

  it('holds 4.5:1 on the raised surface at every hue', () => {
    // The surface keeps the scale's fixed tint whatever the agent's hue is.
    const raised = hsl(190, 0.07, 0.16);
    for (let h = 0; h < 360; h++) {
      const accent = hsl(h, 0.7, accentLightness(h) / 100);
      expect(contrast(accent, raised), `hue ${h}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it('agrees with the tokens in index.css', () => {
    // The module repeats three numbers from the stylesheet (the accent's
    // saturation and base lightness, and the raised surface in dark). If either
    // side moves alone, the contrast guarantee above is about a colour nobody draws.
    // Read from disk (relative to dashboard/, where vitest runs): a CSS import
    // resolves to an empty module under vitest.
    const css = readFileSync('src/index.css', 'utf8');
    const root = css.match(/:root\s*\{[\s\S]*?\n {2}\}/)?.[0] ?? '';
    const dark = css.match(/\.dark\s*\{[\s\S]*?\n {2}\}/)?.[0] ?? '';
    expect(root).toMatch(/--h: 190;/);
    expect(root).toMatch(/--agent: var\(--h\) 70% 58%;/);
    expect(dark).toMatch(/--surface-primary: var\(--h\) 7% 16%;/);
    expect(agentAccentTriplet({ id: 'x' })).toMatch(/^\d+ 70% \d+%$/);
  });

  it('writes the present agent onto the root, and clears it when nobody is present', () => {
    const root = document.createElement('html');
    applyPresentAgent({ id: 'agent.sapphy' }, root);
    expect(root.style.getPropertyValue('--agent')).toBe(agentAccentTriplet({ id: 'agent.sapphy' }));
    // The scale's tint is not the agent's to change: the surfaces stay put.
    expect(root.style.getPropertyValue('--h')).toBe('');

    applyPresentAgent(null, root);
    expect(root.style.getPropertyValue('--agent')).toBe('');
  });
});
