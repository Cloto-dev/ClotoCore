/** Safely parse JSON, returning fallback on failure. */
export function safeJsonParse<T>(str: string, fallback: T): T {
  try { return JSON.parse(str); } catch { return fallback; }
}

/** Try to parse a JSON string as an object. Returns null on failure or non-object results. */
export function tryParseJsonObject(text?: string): Record<string, unknown> | null {
  if (!text) return null;
  try {
    const parsed = JSON.parse(text);
    return typeof parsed === 'object' && parsed !== null ? parsed : null;
  } catch {
    return null;
  }
}
