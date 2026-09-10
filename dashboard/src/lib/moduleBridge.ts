/** The rule a runtime module's request to the kernel has to pass.
 *
 * A module runs in a frame with no origin of its own, so it holds neither the
 * session cookie nor the admin key and cannot reach the kernel by itself. When
 * it needs data it asks the host, and the host decides. That decision is here,
 * as a pure function, because it is the entire access-control story for modules
 * — putting it in the component would mean the only way to test it is to mount
 * a frame and post messages at it.
 *
 * What a module may ask for is what its own `module.json` declared in
 * `requires`. Declaring is not granting on its own: the manifest is written by
 * the module author, so it can only narrow what the host is willing to do,
 * never widen it — hence the method allowlist below, which the manifest cannot
 * override.
 *
 * A declaration is either an exact path or a path ending in `/*`, which stands
 * for one further segment and nothing more. The second shape is still a
 * narrowing of the same floor, not a way past it: the host's floor is any `GET`
 * to an `/api/` path, and both shapes sit inside it. It exists because some
 * modules are written against a route rather than against one row of it — a
 * viewer for whatever a kernel has published cannot name the publisher in
 * advance, and a manifest that had to would be a different module per kernel.
 * The breadth stays visible where it matters: `/api/published/*` reads, in the
 * manifest an operator can open, as "any publisher on this route".
 */

/** Methods the host will proxy at all, whatever a manifest declares.
 *
 * Reads only, for now. Nothing that exists today needs a module to change
 * kernel state, and a module that could would be able to do it without the
 * operator seeing the call — widen this only alongside a decision about how
 * that is surfaced. */
const PROXYABLE_METHODS = new Set(['GET']);

export const MODULE_CALL = 'module.call';
export const MODULE_RESULT = 'module.result';

export interface ModuleCallRequest {
  id: string;
  method: string;
  path: string;
}

export type BridgeDecision =
  | { allowed: true; request: ModuleCallRequest }
  /** `id` is present whenever the message was well-formed enough to answer:
   * a refusal the module cannot correlate with its request is a hang. */
  | { allowed: false; id: string | null; reason: string };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

/** Whether `path` is the one further segment a `.../*` declaration stands for.
 *
 * "One segment" is checked, not assumed. The characters excluded are the ones
 * that would make the tail something other than a single segment of this route:
 * a `/` continues the path, `?` and `#` end it and start something else, and
 * `.` / `..` are the two segments that mean a different place than they read as.
 * Without those checks `/api/published/*` would admit `/api/published/../agents`,
 * which is not a publisher and is exactly the path a manifest reader would think
 * the declaration excluded. */
function matchesSegmentWildcard(pattern: string, path: string): boolean {
  if (!pattern.endsWith('/*')) return false;
  const prefix = pattern.slice(0, -1); // keep the trailing '/'
  if (!path.startsWith(prefix)) return false;
  const tail = path.slice(prefix.length);
  if (tail.length === 0 || tail === '.' || tail === '..') return false;
  return !/[/?#]/.test(tail);
}

/** Whether the manifest declared this call, exactly or by segment wildcard. */
function isDeclared(requires: readonly string[], method: string, path: string): boolean {
  const exact = `${method} ${path}`;
  if (requires.includes(exact)) return true;
  return requires.some((entry) => {
    const space = entry.indexOf(' ');
    if (space < 0) return false;
    if (entry.slice(0, space) !== method) return false;
    return matchesSegmentWildcard(entry.slice(space + 1), path);
  });
}

/** Decide whether one posted message is a call this module is allowed to make.
 *
 * `requires` comes from the module's manifest as `"<METHOD> <path>"` strings. */
export function decideModuleCall(raw: unknown, requires: readonly string[]): BridgeDecision {
  if (!isRecord(raw) || raw.cloto !== MODULE_CALL) {
    return { allowed: false, id: null, reason: 'not a module call' };
  }
  const id = typeof raw.id === 'string' && raw.id.length > 0 ? raw.id : null;
  if (id === null) {
    return { allowed: false, id: null, reason: 'missing request id' };
  }
  const method = typeof raw.method === 'string' ? raw.method.toUpperCase() : '';
  const path = typeof raw.path === 'string' ? raw.path : '';
  if (!method || !path) {
    return { allowed: false, id, reason: 'missing method or path' };
  }
  if (!PROXYABLE_METHODS.has(method)) {
    return { allowed: false, id, reason: `${method} is not proxied for modules` };
  }
  // A relative or scheme-bearing path would let a module aim the host's
  // credential somewhere else entirely.
  if (!path.startsWith('/api/')) {
    return { allowed: false, id, reason: 'path must be an /api/ path' };
  }
  if (!isDeclared(requires, method, path)) {
    return { allowed: false, id, reason: `${method} ${path} is not declared in this module's requires` };
  }
  return { allowed: true, request: { id, method, path } };
}
