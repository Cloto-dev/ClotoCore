/** Get capped device pixel ratio (max 2x to avoid excessive GPU usage) */
export function getDpr(): number {
  return Math.min(window.devicePixelRatio || 1, 2);
}
