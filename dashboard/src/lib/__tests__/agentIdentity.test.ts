import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  accentLightness,
  agentAccentTriplet,
  agentColor,
  agentHue,
  applyPresentAgent,
  parseAccentTriplet,
  setAccentSurface,
} from '../agentIdentity';

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

  it('agrees with the accent token in index.css', () => {
    // The module repeats the accent's saturation and base lightness from the
    // stylesheet. If either side moves alone, the contrast guarantee above is
    // about a colour nobody draws. (The raised surface is no longer repeated:
    // the theme applier hands over the one on screen.)
    // Read from disk (relative to dashboard/, where vitest runs): a CSS import
    // resolves to an empty module under vitest.
    const css = readFileSync('src/index.css', 'utf8');
    const root = css.match(/:root\s*\{[\s\S]*?\n {2}\}/)?.[0] ?? '';
    expect(root).toMatch(/--h: 190;/);
    expect(root).toMatch(/--agent: var\(--h\) 70% 58%;/);
    expect(root).toMatch(/--agent-ink: var\(--h\) 50% 10%;/);
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

  it('leaves the accent to the stylesheet when the theme holds a fixed one', () => {
    const root = document.createElement('html');
    applyPresentAgent({ id: 'agent.sapphy' }, root);
    root.dataset.accent = 'fixed';
    applyPresentAgent({ id: 'agent.sapphy' }, root);
    expect(root.style.getPropertyValue('--agent')).toBe('');
    expect(agentColor({ id: 'agent.sapphy' }, root)).toBe('hsl(var(--agent))');
    delete root.dataset.accent;
    expect(agentColor({ id: 'agent.sapphy' }, root)).toMatch(/^hsl\(\d+ 70% \d+%\)$/);
  });

  it('on a light surface darkens the accent until it reads, and sets white ink on it', () => {
    const root = document.createElement('html');
    setAccentSurface([0.84, 0.85, 0.86]);
    try {
      for (let hue = 0; hue < 360; hue += 15) {
        const [h, s, l] = agentAccentTriplet({ id: 'x', metadata: { accent: `${hue} 70% 58%` } })
          .replace(/%/g, '')
          .split(' ')
          .map(Number);
        expect(l, `hue ${hue}`).toBeLessThan(58);
        expect(contrast(hsl(h, s / 100, l / 100), [0.84, 0.85, 0.86]), `hue ${hue}`).toBeGreaterThanOrEqual(4.5);
      }
      applyPresentAgent({ id: 'x', metadata: { accent: '240 70% 58%' } }, root);
      expect(root.style.getPropertyValue('--agent-ink')).toBe('0 0% 100%');
    } finally {
      setAccentSurface(hsl(190, 0.07, 0.16));
    }
    // Back on the dark surface the stylesheet's dark ink is the better one.
    applyPresentAgent({ id: 'x', metadata: { accent: '190 70% 58%' } }, root);
    expect(root.style.getPropertyValue('--agent-ink')).toBe('');
  });
});

describe('a chosen accent', () => {
  it('is used when the agent carries one this product can draw', () => {
    // Cyan at 58% already holds 4.5:1, so the chosen value survives untouched.
    expect(agentAccentTriplet({ id: 'agent.sapphy', metadata: { accent: '190 70% 58%' } })).toBe('190 70% 58%');
    // And it is not the colour the id would have given this agent.
    expect(agentAccentTriplet({ id: 'agent.sapphy', metadata: { accent: '20 40% 62%' } })).not.toBe(
      agentAccentTriplet({ id: 'agent.sapphy' }),
    );
  });

  it('falls back to the id-derived colour when the stored value is not a colour', () => {
    const own = agentAccentTriplet({ id: 'agent.sapphy' });
    for (const bad of ['', 'blue', '190 70 58', 'hsl(190 70% 58%)', '400 70% 58%', '190 140% 58%', '190 70% 140%']) {
      expect(agentAccentTriplet({ id: 'agent.sapphy', metadata: { accent: bad } }), bad).toBe(own);
      if (bad !== '') expect(parseAccentTriplet(bad), bad).toBeNull();
    }
  });

  it('raises a chosen colour that would not read, and leaves one that already does', () => {
    // Blue at 58% is 2.4:1 on the raised surface; the loop lifts it to where it reads.
    const raised = hsl(190, 0.07, 0.16);
    const chosen = agentAccentTriplet({ id: 'agent.x', metadata: { accent: '240 70% 58%' } });
    const [h, s, l] = chosen.replace(/%/g, '').split(' ').map(Number);
    expect(h).toBe(240);
    expect(s).toBe(70);
    expect(l).toBeGreaterThan(58);
    expect(contrast(hsl(h, s / 100, l / 100), raised)).toBeGreaterThanOrEqual(4.5);
  });

  it('holds 4.5:1 for a chosen colour at every hue', () => {
    const raised = hsl(190, 0.07, 0.16);
    for (let h = 0; h < 360; h += 5) {
      const triplet = agentAccentTriplet({ id: 'agent.y', metadata: { accent: `${h} 70% 30%` } });
      const [hh, ss, ll] = triplet.replace(/%/g, '').split(' ').map(Number);
      expect(contrast(hsl(hh, ss / 100, ll / 100), raised), `hue ${h}`).toBeGreaterThanOrEqual(4.5);
    }
  });
});
