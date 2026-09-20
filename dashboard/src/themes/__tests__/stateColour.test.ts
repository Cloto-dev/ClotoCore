/**
 * The state colour: something failed, is broken, or will be destroyed.
 *
 * It used to be written out by hand — `hsl(6 70% 72%)` in twenty-one places,
 * with three near-misses beside it (`6 70% 68%`, `6 70% 62%`, `0 70% 62%`) that
 * are the same signal and read as drift rather than intent. One value cannot
 * sit on both faces: on the light surfaces the dark one reads under 2:1, which
 * is not a colour, it is a stain.
 *
 * These hold the three things that made it worth doing: that the value is a
 * token, that each face's value is legible on that face's surfaces, and that
 * nothing has quietly written the raw colour back.
 */

import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { contrastOf, type Hsl, MIN_CONTRAST } from '../validate';

const css = readFileSync('src/index.css', 'utf8');

/** `--state-bad` as declared inside one face's block. */
function stateBad(selector: string): Hsl {
  const start = css.indexOf(`\n  ${selector} {`);
  expect(start, `${selector} block`).toBeGreaterThan(-1);
  const body = css.slice(start, css.indexOf('\n  }', start));
  const m = /--state-bad: ([\d.]+) ([\d.]+)% ([\d.]+)%;/.exec(body);
  expect(m, `--state-bad inside ${selector}`).not.toBeNull();
  return [Number(m?.[1]), Number(m?.[2]), Number(m?.[3])];
}

/**
 * The three surfaces a face draws on. They carry the present agent's hue, so
 * the whole circle is checked rather than the one hue that happens to be set:
 * a colour that is legible only beside some agents is not legible.
 */
const LIGHT_SURFACES: Array<[number, number]> = [
  [6, 92], // base
  [7, 88], // secondary
  [7, 84], // primary
];
const DARK_SURFACES: Array<[number, number]> = [
  [6, 8],
  [7, 12],
  [7, 16],
];
const HUES = Array.from({ length: 12 }, (_, i) => i * 30);

function worstContrast(colour: Hsl, surfaces: Array<[number, number]>): number {
  let worst = Number.POSITIVE_INFINITY;
  for (const [s, l] of surfaces) {
    for (const h of HUES) worst = Math.min(worst, contrastOf(colour, [h, s, l]));
  }
  return worst;
}

describe('the state colour', () => {
  it('is declared for both faces', () => {
    expect(stateBad(':root')).toBeTruthy();
    expect(stateBad('.dark')).toBeTruthy();
  });

  it('is legible on every surface of its own face, at every agent hue', () => {
    // The measurement the change exists for. Run with the project's own
    // formula, so this cannot disagree with what the theme validator says.
    expect(worstContrast(stateBad(':root'), LIGHT_SURFACES)).toBeGreaterThanOrEqual(MIN_CONTRAST);
    expect(worstContrast(stateBad('.dark'), DARK_SURFACES)).toBeGreaterThanOrEqual(MIN_CONTRAST);
  });

  it('is a different value per face, because one value cannot serve both', () => {
    // Guards the mistake this replaces: the dark value on a light surface reads
    // under 2:1. If the two faces are ever given the same value again, one of
    // them is unreadable.
    expect(stateBad(':root')).not.toEqual(stateBad('.dark'));
    expect(worstContrast(stateBad('.dark'), LIGHT_SURFACES)).toBeLessThan(2);
  });

  it('keeps the dark face exactly as it was drawn', () => {
    // Twenty-one places said this. Changing it here would restyle the product
    // under cover of a refactor.
    expect(stateBad('.dark')).toEqual([6, 70, 72]);
  });
});

describe('nothing writes the state colour by hand', () => {
  /** Every stylesheet under src/, read from disk. */
  function stylesheets(): Array<[string, string]> {
    const out: Array<[string, string]> = [];
    const walk = (dir: string) => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const full = join(dir, entry.name);
        if (entry.isDirectory()) walk(full);
        else if (entry.name.endsWith('.css')) out.push([full, readFileSync(full, 'utf8')]);
      }
    };
    walk('src');
    return out;
  }
  const sheets = stylesheets();

  it('finds the stylesheets it is meant to be reading, with something in them', () => {
    // Both halves matter. An earlier version of this file counted files and not
    // their contents: the reader returned empty strings, the rule below passed
    // against nothing, and only the "is actually used" check noticed.
    expect(sheets.length).toBeGreaterThan(5);
    const chars = sheets.reduce((n, [, text]) => n + text.length, 0);
    expect(chars).toBeGreaterThan(10_000);
  });

  it('leaves no raw red in a component stylesheet', () => {
    const offenders: string[] = [];
    for (const [path, text] of sheets) {
      if (path.endsWith('index.css') || path.endsWith('compiled-tailwind.css')) continue;
      for (const m of text.matchAll(/hsl\(\s*[06]\s+\d+%\s+\d+%\s*\)/g)) offenders.push(`${path}: ${m[0]}`);
    }
    expect(offenders, 'use hsl(var(--state-bad))').toEqual([]);
  });

  it('catches a raw red that is put back', () => {
    // The detector, on a known positive: without this, a regex that matches
    // nothing would look exactly like a codebase that is clean.
    const planted = '.x { color: hsl(6 70% 72%); }';
    expect([...planted.matchAll(/hsl\(\s*[06]\s+\d+%\s+\d+%\s*\)/g)]).toHaveLength(1);
  });

  it('is actually used, so the rule above is not vacuous', () => {
    const uses = sheets.reduce((n, [, text]) => n + [...text.matchAll(/var\(--state-bad\)/g)].length, 0);
    expect(uses).toBeGreaterThanOrEqual(20);
  });
});

describe('the token reaches the browser', () => {
  // index.css is the Tailwind input; main.tsx imports only the compiled sheet.
  // A token added to the input and not compiled is a token that does not exist
  // at runtime, and every rule using it silently loses its colour.
  it('is in the compiled stylesheet, for both faces', () => {
    const compiled = readFileSync('src/compiled-tailwind.css', 'utf8');
    expect([...compiled.matchAll(/--state-bad:/g)]).toHaveLength(2);
  });
});
