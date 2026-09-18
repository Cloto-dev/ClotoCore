/**
 * Where the console keeps an agent's reasoning trace between mounts.
 *
 * The console owns the trace; the roster only reads the last line of it to say
 * what a running agent is doing. The key lives here so the reader and the
 * writer cannot drift apart — a copied key would keep compiling and quietly
 * read nothing.
 */

export function thinkingStorageKey(agentId: string): string {
  return `cloto-thinking-${agentId}`;
}

/** The text of the most recent step, or null when there is no trace to read. */
export function latestThinkingText(agentId: string): string | null {
  try {
    const raw = sessionStorage.getItem(thinkingStorageKey(agentId));
    if (!raw) return null;
    const steps = JSON.parse(raw);
    if (!Array.isArray(steps) || steps.length === 0) return null;
    const text = steps[steps.length - 1]?.text;
    return typeof text === 'string' && text.trim() ? text.trim() : null;
  } catch {
    return null;
  }
}
