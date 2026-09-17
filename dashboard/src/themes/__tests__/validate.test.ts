import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { contrastOf, MAX_PACK_BYTES, parseTriplet, parseTripletAlpha, validateThemePack } from '../validate';

const FACE = {
  'surface-base': '200 10% 8%',
  'surface-secondary': '200 10% 12%',
  'surface-primary': '200 10% 16%',
  'border-default': '200 10% 22%',
  'border-subtle': '200 10% 16%',
  'surface-overlay': '0 0% 0% / 0.7',
  'text-primary': '200 10% 92%',
  'text-secondary': '200 8% 68%',
  'text-tertiary': '200 6% 62%',
  'text-muted': '200 6% 40%',
};

function pack(overrides: Record<string, unknown> = {}): string {
  return JSON.stringify({ schema: 1, id: 'probe', label: 'Probe', dark: FACE, ...overrides });
}

function errorsOf(json: string): string[] {
  const result = validateThemePack(json);
  return result.ok ? [] : result.errors;
}

describe('a theme pack that is accepted', () => {
  it('reads a one-face pack into numbers, with the agent as its accent', () => {
    const result = validateThemePack(pack());
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.theme.id).toBe('probe');
    expect(result.theme.accent).toBeNull();
    expect(result.theme.faces.light).toBeUndefined();
    expect(result.theme.faces.dark?.colours['surface-base']).toEqual([200, 10, 8]);
    expect(result.theme.faces.dark?.overlay).toEqual([0, 0, 0, 0.7]);
    expect(result.warnings).toEqual([]);
  });

  it('reads a role colour when the pack gives one, and leaves it out when it does not', () => {
    const result = validateThemePack(pack({ dark: { ...FACE, 'surface-panel': '10 20% 30%' } }));
    expect(result.ok && result.theme.faces.dark?.colours['surface-panel']).toEqual([10, 20, 30]);
    const plain = validateThemePack(pack());
    expect(plain.ok && 'surface-panel' in (plain.theme.faces.dark?.colours ?? {})).toBe(false);
  });

  it('reads a fixed accent for the faces the pack has', () => {
    const result = validateThemePack(pack({ accent: { dark: { agent: '229 78% 60%', 'agent-ink': '0 0% 0%' } } }));
    expect(result.ok && result.theme.accent).toEqual({ dark: { agent: [229, 78, 60], ink: [0, 0, 0] } });
  });
});

describe('a theme pack that is refused', () => {
  it('names the required token that is missing', () => {
    const { 'text-tertiary': _dropped, ...rest } = FACE;
    expect(errorsOf(pack({ dark: rest }))).toEqual(['dark: missing "text-tertiary"']);
  });

  it('names a missing overlay', () => {
    const { 'surface-overlay': _dropped, ...rest } = FACE;
    expect(errorsOf(pack({ dark: rest }))).toEqual(['dark: missing "surface-overlay"']);
  });

  it('refuses a token it does not know rather than dropping it', () => {
    expect(errorsOf(pack({ dark: { ...FACE, 'radius-tool': '2px' } }))).toEqual(['dark: unknown token "radius-tool"']);
  });

  it('refuses a top-level key it does not know', () => {
    expect(errorsOf(pack({ css: 'body{display:none}' }))).toEqual(['unknown key "css"']);
  });

  it('refuses a value that is not a colour triplet', () => {
    const hostile = '0 0% 0%; } body { display: none } :root {';
    expect(errorsOf(pack({ dark: { ...FACE, 'surface-base': hostile } }))).toEqual([
      'dark: "surface-base" is not an "H S% L%" colour',
    ]);
  });

  it('refuses a colour outside the HSL ranges rather than clamping it', () => {
    expect(parseTriplet('361 10% 10%')).toBeNull();
    expect(parseTriplet('10 101% 10%')).toBeNull();
    expect(parseTriplet('10 10% 101%')).toBeNull();
    expect(parseTriplet('-1 10% 10%')).toBeNull();
    expect(parseTriplet('360 100% 100%')).toEqual([360, 100, 100]);
    expect(parseTripletAlpha('0 0% 0% / 1.1')).toBeNull();
    expect(parseTripletAlpha('0 0% 0% / 1')).toEqual([0, 0, 0, 1]);
  });

  it('refuses an id that could not be a filename, a missing label, and another schema', () => {
    expect(errorsOf(pack({ id: '../evil' }))).toEqual(['"id" must be 1–40 characters of a–z, 0–9 and "-"']);
    expect(errorsOf(pack({ id: 'Probe' }))).toHaveLength(1);
    expect(errorsOf(pack({ label: '  ' }))).toEqual(['"label" must be 1–60 characters']);
    expect(errorsOf(pack({ schema: 2 }))).toEqual(['"schema" is not 1']);
  });

  it('refuses a pack with no face, a file that is not JSON, and one that is too large', () => {
    expect(errorsOf(JSON.stringify({ schema: 1, id: 'probe', label: 'Probe' }))).toEqual([
      'the pack has neither a "light" nor a "dark" face',
    ]);
    expect(errorsOf('{')).toEqual(['the file is not JSON']);
    expect(errorsOf(pack({ author: 'x'.repeat(MAX_PACK_BYTES) }))).toEqual([
      `the file is larger than ${MAX_PACK_BYTES} bytes`,
    ]);
  });

  it('refuses a fixed accent that leaves one of the pack faces without colours', () => {
    const json = pack({
      light: {
        ...FACE,
        'text-primary': '200 10% 2%',
        'surface-base': '200 10% 98%',
        'surface-secondary': '200 10% 96%',
        'surface-primary': '200 10% 94%',
        'text-secondary': '200 10% 20%',
        'text-tertiary': '200 10% 25%',
      },
      accent: { dark: { agent: '229 78% 60%', 'agent-ink': '0 0% 0%' } },
    });
    expect(errorsOf(json)).toEqual(['accent: no colours for the "light" face']);
  });
});

describe('contrast', () => {
  it('refuses primary text that cannot be read on any one of the three surfaces', () => {
    // Readable on base (L 8%) and secondary, not on a raised surface at L 50%.
    const errors = errorsOf(
      pack({
        dark: {
          ...FACE,
          'surface-primary': '200 10% 50%',
          'text-secondary': '200 8% 95%',
          'text-tertiary': '200 8% 95%',
        },
      }),
    );
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatch(/^dark: text-primary on surface-primary \d\.\d\d:1$/);
  });

  it('lists the pack and warns when secondary or tertiary text is faint', () => {
    const result = validateThemePack(pack({ dark: { ...FACE, 'text-tertiary': '200 6% 40%' } }));
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.warnings.map((w) => w.replace(/ \d+\.\d\d:1$/, ''))).toEqual([
      'dark: text-tertiary on surface-base',
      'dark: text-tertiary on surface-secondary',
      'dark: text-tertiary on surface-primary',
    ]);
  });

  it('does not check muted text, which is not for reading', () => {
    const result = validateThemePack(pack({ dark: { ...FACE, 'text-muted': FACE['surface-base'] } }));
    expect(result.ok && result.warnings).toEqual([]);
  });

  it('warns when a fixed accent cannot carry its ink', () => {
    const result = validateThemePack(pack({ accent: { dark: { agent: '229 78% 60%', 'agent-ink': '229 78% 65%' } } }));
    expect(result.ok && result.warnings).toHaveLength(1);
    expect(result.ok && result.warnings[0]).toMatch(/^dark: agent-ink on agent /);
  });

  it('measures the WCAG ratio', () => {
    expect(contrastOf([0, 0, 0], [0, 0, 100])).toBeCloseTo(21, 5);
    expect(contrastOf([0, 0, 50], [0, 0, 50])).toBeCloseTo(1, 5);
  });
});

describe('the bundled packs', () => {
  const dir = 'src/themes/packs';
  const files = readdirSync(dir)
    .filter((f) => f.endsWith('.json'))
    .sort();

  // Every pair under 4.5:1 in a bundled pack is written down here. Legacy keeps
  // the colours it had before the redesign, faint ones included; a new faint
  // pair in any bundled pack has to be added to this list on purpose.
  const KNOWN_LOW_CONTRAST: Record<string, string[]> = {
    'default.json': [],
    'legacy.json': [
      'light: text-tertiary on surface-base',
      'light: text-secondary on surface-secondary',
      'light: text-tertiary on surface-secondary',
      'light: text-tertiary on surface-primary',
      'dark: agent-ink on agent',
    ],
  };

  it('are the ones this test knows about', () => {
    expect(files).toEqual(Object.keys(KNOWN_LOW_CONTRAST).sort());
  });

  it('all validate, carry both faces, and are named after their id', () => {
    for (const file of files) {
      const result = validateThemePack(readFileSync(`${dir}/${file}`, 'utf8'));
      expect(result.ok, file).toBe(true);
      if (!result.ok) continue;
      expect(`${result.theme.id}.json`).toBe(file);
      expect(Object.keys(result.theme.faces).sort()).toEqual(['dark', 'light']);
    }
  });

  it('have exactly the faint pairs written down above', () => {
    for (const file of files) {
      const result = validateThemePack(readFileSync(`${dir}/${file}`, 'utf8'));
      const pairs = result.ok ? result.warnings.map((w) => w.replace(/ \d+\.\d\d:1$/, '')) : ['invalid'];
      expect(pairs, file).toEqual(KNOWN_LOW_CONTRAST[file]);
    }
  });
});
