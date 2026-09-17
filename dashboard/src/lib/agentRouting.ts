/**
 * The other engine an agent's routing can send a turn to, if its rules name
 * one. A rule's `fallback` is the explicit answer to "and if that engine is
 * not there"; when no rule carries one, the first rule's target is the only
 * other engine the routing can reach.
 */
export function firstRoutingAlternative(routing: string | undefined): string | null {
  if (!routing) return null;
  try {
    const rules = JSON.parse(routing);
    if (!Array.isArray(rules)) return null;
    const withFallback = rules.find((r) => typeof r?.fallback === 'string' && r.fallback);
    if (withFallback) return withFallback.fallback;
    const withEngine = rules.find((r) => typeof r?.engine === 'string' && r.engine);
    return withEngine ? withEngine.engine : null;
  } catch {
    return null;
  }
}
