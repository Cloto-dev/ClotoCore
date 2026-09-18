/**
 * The one reader of a theme pack (docs/THEME_PACKS_DESIGN.md §2 (a), (e), (f)).
 *
 * A pack is data someone outside the project wrote. Nothing in it reaches the
 * stylesheet as written: every colour is parsed into numbers here, and
 * `apply.ts` writes the stylesheet from those numbers. The loader, the import
 * button and the test over the bundled packs all go through `validateThemePack`,
 * so there is no second implementation to drift from this one.
 */

/** Hue 0–360, saturation and lightness 0–100. */
export type Hsl = [number, number, number];
/** A colour with an alpha 0–1 — only `surface-overlay` has one. */
export type Hsla = [number, number, number, number];

/** The colours a face has to define. */
export const REQUIRED_TOKENS = [
  'surface-base',
  'surface-secondary',
  'surface-primary',
  'border-default',
  'border-subtle',
  'text-primary',
  'text-secondary',
  'text-tertiary',
  'text-muted',
] as const;
/** Required like the ones above, and the only colour that carries an alpha. */
export const OVERLAY_TOKEN = 'surface-overlay';
/** Role names. Omitted, each resolves to the step it resolves to in index.css. */
export const OPTIONAL_TOKENS = ['surface-panel', 'surface-field', 'surface-control', 'surface-card'] as const;

export type ColourToken = (typeof REQUIRED_TOKENS)[number] | (typeof OPTIONAL_TOKENS)[number];
export type FaceName = 'light' | 'dark';
export const FACES: readonly FaceName[] = ['light', 'dark'];

export interface ThemeFace {
  colours: Partial<Record<ColourToken, Hsl>> & Record<(typeof REQUIRED_TOKENS)[number], Hsl>;
  overlay: Hsla;
}

export interface FixedAccent {
  agent: Hsl;
  ink: Hsl;
}

export interface Theme {
  id: string;
  label: string;
  author?: string;
  version?: string;
  license?: string;
  /** `null`: the accent is the present agent's colour. Otherwise one fixed
   * accent per face the pack has. */
  accent: null | Partial<Record<FaceName, FixedAccent>>;
  faces: Partial<Record<FaceName, ThemeFace>>;
}

export type ThemeValidation = { ok: true; theme: Theme; warnings: string[] } | { ok: false; errors: string[] };

/** A pack larger than this is not read. The bundled ones are under 2 KB. */
export const MAX_PACK_BYTES = 16 * 1024;
export const SCHEMA_VERSION = 1;
/** The contrast body text has to hold (WCAG AA). */
export const MIN_CONTRAST = 4.5;

const ID_PATTERN = /^[a-z0-9-]{1,40}$/;
const TOP_LEVEL_KEYS = new Set(['schema', 'id', 'label', 'author', 'version', 'license', 'accent', 'light', 'dark']);
const SURFACE_STEPS = ['surface-base', 'surface-secondary', 'surface-primary'] as const;
const NUMBER = String.raw`(\d+(?:\.\d+)?)`;
const TRIPLET = new RegExp(String.raw`^\s*${NUMBER}\s+${NUMBER}%\s+${NUMBER}%\s*$`);
const TRIPLET_ALPHA = new RegExp(String.raw`^\s*${NUMBER}\s+${NUMBER}%\s+${NUMBER}%\s*/\s*${NUMBER}\s*$`);

function inRange(h: number, s: number, l: number): boolean {
  return h >= 0 && h <= 360 && s >= 0 && s <= 100 && l >= 0 && l <= 100;
}

/** `"H S% L%"` as numbers, or `null`. Out-of-range values are refused, not clamped. */
export function parseTriplet(raw: unknown): Hsl | null {
  if (typeof raw !== 'string') return null;
  const m = TRIPLET.exec(raw);
  if (!m) return null;
  const [h, s, l] = [Number(m[1]), Number(m[2]), Number(m[3])];
  return inRange(h, s, l) ? [h, s, l] : null;
}

/** `"H S% L% / A"` as numbers, or `null`. */
export function parseTripletAlpha(raw: unknown): Hsla | null {
  if (typeof raw !== 'string') return null;
  const m = TRIPLET_ALPHA.exec(raw);
  if (!m) return null;
  const [h, s, l, a] = [Number(m[1]), Number(m[2]), Number(m[3]), Number(m[4])];
  return inRange(h, s, l) && a >= 0 && a <= 1 ? [h, s, l, a] : null;
}

/** sRGB channels (0–1) of an HSL colour given in degrees and percentages. */
export function hslToRgb([h, sPct, lPct]: Hsl): [number, number, number] {
  const s = sPct / 100;
  const l = lPct / 100;
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => {
    const k = (n + h / 30) % 12;
    return l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
  };
  return [f(0), f(8), f(4)];
}

function relativeLuminance([r, g, b]: [number, number, number]): number {
  const lin = (c: number) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** WCAG contrast ratio of two HSL colours. */
export function contrastOf(a: Hsl, b: Hsl): number {
  const [hi, lo] = [relativeLuminance(hslToRgb(a)), relativeLuminance(hslToRgb(b))].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function readFace(name: FaceName, raw: unknown, errors: string[]): ThemeFace | null {
  if (!isRecord(raw)) {
    errors.push(`"${name}" is not an object`);
    return null;
  }
  const known = new Set<string>([...REQUIRED_TOKENS, ...OPTIONAL_TOKENS, OVERLAY_TOKEN]);
  for (const key of Object.keys(raw)) {
    if (!known.has(key)) errors.push(`${name}: unknown token "${key}"`);
  }
  const colours: Partial<Record<ColourToken, Hsl>> = {};
  for (const token of REQUIRED_TOKENS) {
    if (!(token in raw)) {
      errors.push(`${name}: missing "${token}"`);
      continue;
    }
    const value = parseTriplet(raw[token]);
    if (value) colours[token] = value;
    else errors.push(`${name}: "${token}" is not an "H S% L%" colour`);
  }
  for (const token of OPTIONAL_TOKENS) {
    if (!(token in raw)) continue;
    const value = parseTriplet(raw[token]);
    if (value) colours[token] = value;
    else errors.push(`${name}: "${token}" is not an "H S% L%" colour`);
  }
  let overlay: Hsla | null = null;
  if (!(OVERLAY_TOKEN in raw)) errors.push(`${name}: missing "${OVERLAY_TOKEN}"`);
  else {
    overlay = parseTripletAlpha(raw[OVERLAY_TOKEN]);
    if (!overlay) errors.push(`${name}: "${OVERLAY_TOKEN}" is not an "H S% L% / A" colour`);
  }
  if (!overlay || REQUIRED_TOKENS.some((t) => !colours[t])) return null;
  return { colours: colours as ThemeFace['colours'], overlay };
}

function readAccent(raw: unknown, faces: Theme['faces'], errors: string[]): Theme['accent'] {
  if (raw === undefined || raw === 'agent') return null;
  if (!isRecord(raw)) {
    errors.push('"accent" is neither "agent" nor an object');
    return null;
  }
  const accent: Partial<Record<FaceName, FixedAccent>> = {};
  for (const key of Object.keys(raw)) {
    if (!FACES.includes(key as FaceName)) errors.push(`accent: unknown face "${key}"`);
  }
  for (const face of FACES) {
    if (!faces[face]) continue;
    const entry = raw[face];
    if (!isRecord(entry)) {
      errors.push(`accent: no colours for the "${face}" face`);
      continue;
    }
    for (const key of Object.keys(entry)) {
      if (key !== 'agent' && key !== 'agent-ink') errors.push(`accent.${face}: unknown key "${key}"`);
    }
    const agent = parseTriplet(entry.agent);
    const ink = parseTriplet(entry['agent-ink']);
    if (!agent) errors.push(`accent.${face}: "agent" is not an "H S% L%" colour`);
    if (!ink) errors.push(`accent.${face}: "agent-ink" is not an "H S% L%" colour`);
    if (agent && ink) accent[face] = { agent, ink };
  }
  return accent;
}

/** A pair that reads under 4.5:1, as `"<face>: <text> on <surface> <ratio>"`. */
function lowContrast(face: FaceName, text: string, on: string, ratio: number): string {
  return `${face}: ${text} on ${on} ${ratio.toFixed(2)}:1`;
}

function checkContrast(theme: Theme, errors: string[], warnings: string[]) {
  for (const name of FACES) {
    const face = theme.faces[name];
    if (!face) continue;
    for (const surface of SURFACE_STEPS) {
      // The lock-out guard: under a theme whose primary text cannot be read, the
      // settings page that would switch away from it cannot be read either.
      const primary = contrastOf(face.colours['text-primary'], face.colours[surface]);
      if (primary < MIN_CONTRAST) errors.push(lowContrast(name, 'text-primary', surface, primary));
      // `text-muted` is not checked: it is defined as not for reading.
      for (const text of ['text-secondary', 'text-tertiary'] as const) {
        const ratio = contrastOf(face.colours[text], face.colours[surface]);
        if (ratio < MIN_CONTRAST) warnings.push(lowContrast(name, text, surface, ratio));
      }
    }
    const fixed = theme.accent?.[name];
    if (fixed) {
      const ratio = contrastOf(fixed.ink, fixed.agent);
      if (ratio < MIN_CONTRAST) warnings.push(lowContrast(name, 'agent-ink', 'agent', ratio));
    }
  }
}

function optionalString(raw: Record<string, unknown>, key: string, errors: string[]): string | undefined {
  const value = raw[key];
  if (value === undefined) return undefined;
  if (typeof value !== 'string' || value.length > 200) {
    errors.push(`"${key}" is not a short string`);
    return undefined;
  }
  return value;
}

/** Read a pack from its JSON text. */
export function validateThemePack(json: string): ThemeValidation {
  if (new TextEncoder().encode(json).length > MAX_PACK_BYTES) {
    return { ok: false, errors: [`the file is larger than ${MAX_PACK_BYTES} bytes`] };
  }
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return { ok: false, errors: ['the file is not JSON'] };
  }
  if (!isRecord(raw)) return { ok: false, errors: ['the file is not a JSON object'] };

  const errors: string[] = [];
  for (const key of Object.keys(raw)) {
    if (!TOP_LEVEL_KEYS.has(key)) errors.push(`unknown key "${key}"`);
  }
  if (raw.schema !== SCHEMA_VERSION) errors.push(`"schema" is not ${SCHEMA_VERSION}`);
  const id = typeof raw.id === 'string' && ID_PATTERN.test(raw.id) ? raw.id : null;
  if (!id) errors.push('"id" must be 1–40 characters of a–z, 0–9 and "-"');
  const label = typeof raw.label === 'string' && raw.label.trim() && raw.label.length <= 60 ? raw.label : null;
  if (!label) errors.push('"label" must be 1–60 characters');
  const author = optionalString(raw, 'author', errors);
  const version = optionalString(raw, 'version', errors);
  const license = optionalString(raw, 'license', errors);

  const faces: Theme['faces'] = {};
  for (const name of FACES) {
    if (!(name in raw)) continue;
    const face = readFace(name, raw[name], errors);
    if (face) faces[name] = face;
  }
  if (!('light' in raw) && !('dark' in raw)) errors.push('the pack has neither a "light" nor a "dark" face');
  const accent = readAccent(raw.accent, faces, errors);

  if (errors.length > 0 || !id || !label) return { ok: false, errors };
  const theme: Theme = { id, label, author, version, license, accent, faces };
  const warnings: string[] = [];
  checkContrast(theme, errors, warnings);
  if (errors.length > 0) return { ok: false, errors };
  return { ok: true, theme, warnings };
}
