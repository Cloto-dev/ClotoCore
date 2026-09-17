import { User } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useApi } from '../hooks/useApi';
import type { AgentMetadata } from '../types';

/** The agent's hue, 0–359. Derived from the id, which does not change when the
 * agent is renamed. */
export function agentHue(agent: Pick<AgentMetadata, 'id'>): number {
  // FNV-1a: spreads ids that differ by one character across the circle.
  let hash = 0x811c9dc5;
  for (let i = 0; i < agent.id.length; i++) {
    hash ^= agent.id.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0) % 360;
}

/** sRGB channels (0–1) of an HSL colour. */
function hslToRgb(h: number, s: number, l: number): [number, number, number] {
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

export function contrastRatio(a: [number, number, number], b: [number, number, number]): number {
  const [hi, lo] = [relativeLuminance(a), relativeLuminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const ACCENT_SATURATION = 0.7;
const ACCENT_BASE_LIGHTNESS = 0.58;
/** The raised surface in dark (`--surface-primary` in index.css): the lightest
 * surface the accent is set on as text. Its hue is the scale's fixed tint. */
const SCALE_HUE = 190;
const RAISED_SATURATION = 0.07;
const RAISED_LIGHTNESS = 0.16;
const MIN_CONTRAST = 4.5;

/** Lightness of the accent at this hue. A fixed 58% reads at cyan (7:1) but not
 * at blue (2.4:1), so the lightness rises until the accent holds 4.5:1 on the
 * raised surface. */
export function accentLightness(hue: number): number {
  const raised = hslToRgb(SCALE_HUE, RAISED_SATURATION, RAISED_LIGHTNESS);
  let l = ACCENT_BASE_LIGHTNESS;
  while (l < 0.9 && contrastRatio(hslToRgb(hue, ACCENT_SATURATION, l), raised) < MIN_CONTRAST) {
    l += 0.01;
  }
  return Math.round(l * 100);
}

/** An agent whose accent can be read: the id always, the metadata when the
 * caller has it. Call sites that only know an id pass just the id. */
export type AccentSource = Pick<AgentMetadata, 'id'> & Partial<Pick<AgentMetadata, 'metadata'>>;

/**
 * A chosen accent, as three HSL components, or `null` when the stored value is
 * not one this product can draw.
 *
 * The stored form is the bare triplet (`"190 70% 58%"`) so it can be dropped
 * straight into `--agent`, which is what every consumer of the accent reads.
 * Anything outside the HSL ranges is refused rather than clamped: a value the
 * person cannot have meant is better answered with the agent's own colour than
 * with a nearby one they did not choose.
 */
export function parseAccentTriplet(raw: string | undefined): [number, number, number] | null {
  if (!raw) return null;
  const m = /^\s*(-?[\d.]+)\s+(-?[\d.]+)%\s+(-?[\d.]+)%\s*$/.exec(raw);
  if (!m) return null;
  const [h, s, l] = [Number(m[1]), Number(m[2]), Number(m[3])];
  if (![h, s, l].every(Number.isFinite)) return null;
  if (h < 0 || h > 360) return null;
  if (s < 0 || s > 100) return null;
  if (l < 0 || l > 100) return null;
  return [h, s, l];
}

/**
 * Lightness of a chosen accent, raised until it holds 4.5:1 on the raised
 * surface. The same loop the id-derived colour goes through: choosing a colour
 * is allowed, choosing an unreadable one is not.
 */
function readableLightness(hue: number, saturation: number, lightness: number): number {
  const raised = hslToRgb(SCALE_HUE, RAISED_SATURATION, RAISED_LIGHTNESS);
  let l = lightness / 100;
  while (l < 0.9 && contrastRatio(hslToRgb(hue, saturation / 100, l), raised) < MIN_CONTRAST) {
    l += 0.01;
  }
  return Math.round(l * 100);
}

/** The `--agent` token value for an agent: an HSL component triplet. The
 * agent's own `metadata.accent` when it holds one this product can draw, else
 * the colour its id gives it. */
export function agentAccentTriplet(agent: AccentSource): string {
  const chosen = parseAccentTriplet(agent.metadata?.accent);
  if (chosen) {
    const [h, s, l] = chosen;
    return `${h} ${s}% ${readableLightness(h, s, l)}%`;
  }
  const hue = agentHue(agent);
  return `${hue} ${ACCENT_SATURATION * 100}% ${accentLightness(hue)}%`;
}

/** The accent colour of an agent, as a CSS colour. Under the Legacy theme every
 * agent wears the old brand blue, which the stylesheet holds in `--agent`. */
export function agentColor(agent: AccentSource, root: HTMLElement = document.documentElement): string {
  if (root.classList.contains('theme-legacy')) return 'hsl(var(--agent))';
  return `hsl(${agentAccentTriplet(agent)})`;
}

/** Make `agent` the one present: its colour becomes the accent
 * (docs/DESIGN_PHILOSOPHY.md §4.2). The neutral scale is not touched — its tint
 * is fixed, so the surfaces do not change when the selection does. `null` falls
 * back to the default in index.css. */
export function applyPresentAgent(agent: AccentSource | null, root: HTMLElement = document.documentElement) {
  if (!agent) {
    root.style.removeProperty('--agent');
    return;
  }
  root.style.setProperty('--agent', agentAccentTriplet(agent));
}

/** Render the appropriate icon for an agent (avatar image or fallback icon) */
export function AgentIcon({ agent, size = 20 }: { agent: AgentMetadata; size?: number }) {
  const api = useApi();
  const [imgError, setImgError] = useState(false);
  // Cache-bust using avatar_updated_at (set by backend on every upload).
  // For legacy avatars without the timestamp, use a mount-time value so
  // each fresh render cycle fetches the latest image.
  const mountKey = useMemo(() => Date.now().toString(), []);
  const avatarVersion = agent.metadata?.avatar_updated_at ?? mountKey;

  if (agent.metadata?.has_avatar === 'true' && !imgError) {
    return (
      <img
        src={api.getAvatarUrl(agent.id, avatarVersion)}
        alt={agent.name}
        className="rounded-md object-cover"
        style={{ width: size, height: size }}
        onError={() => setImgError(true)}
      />
    );
  }
  return <User size={size} />;
}

/** Status dot color classes (3-state) */
export function statusDotColor(status: string): string {
  return status === 'online'
    ? 'bg-emerald-500'
    : status === 'degraded'
      ? 'bg-amber-500 animate-pulse'
      : 'bg-content-muted';
}

/** Status badge classes (3-state) */
export function statusBadgeClass(status: string): string {
  return status === 'online'
    ? 'bg-emerald-500/10 text-emerald-500'
    : status === 'degraded'
      ? 'bg-amber-500/10 text-amber-500'
      : 'bg-surface-secondary text-content-tertiary';
}
