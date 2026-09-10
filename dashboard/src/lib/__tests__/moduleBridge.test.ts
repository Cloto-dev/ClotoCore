import { describe, expect, it } from 'vitest';
import { decideModuleCall, MODULE_CALL } from '../moduleBridge';

const call = (over: Record<string, unknown> = {}) => ({
  cloto: MODULE_CALL,
  id: 'r1',
  method: 'GET',
  path: '/api/system/health',
  ...over,
});

const REQUIRES = ['GET /api/system/health'];

describe('decideModuleCall', () => {
  it('allows exactly what the manifest declared', () => {
    const decision = decideModuleCall(call(), REQUIRES);
    expect(decision.allowed).toBe(true);
    if (decision.allowed) {
      expect(decision.request).toEqual({ id: 'r1', method: 'GET', path: '/api/system/health' });
    }
  });

  it('refuses a path the manifest did not declare', () => {
    // The interesting case: a real, working kernel route that this module
    // simply never asked for. Declaring one path must not open the surface.
    const decision = decideModuleCall(call({ path: '/api/agents' }), REQUIRES);
    expect(decision.allowed).toBe(false);
    if (!decision.allowed) {
      expect(decision.id).toBe('r1');
      expect(decision.reason).toContain('not declared');
    }
  });

  it('refuses a method the host does not proxy, even when declared', () => {
    // A manifest can narrow what the host will do; it cannot widen it.
    const decision = decideModuleCall(call({ method: 'DELETE', path: '/api/agents/x' }), ['DELETE /api/agents/x']);
    expect(decision.allowed).toBe(false);
    if (!decision.allowed) expect(decision.reason).toContain('not proxied');
  });

  it('refuses a path that is not a kernel API path', () => {
    for (const path of ['https://example.com/api/system/health', '../api/system/health', '/etc/passwd']) {
      const decision = decideModuleCall(call({ path }), [`GET ${path}`]);
      expect(decision.allowed, path).toBe(false);
      if (!decision.allowed) expect(decision.reason).toContain('/api/');
    }
  });

  it('ignores messages that are not module calls', () => {
    for (const raw of [null, 'hello', 42, {}, { cloto: 'something.else', id: 'r1' }]) {
      const decision = decideModuleCall(raw, REQUIRES);
      expect(decision.allowed).toBe(false);
      if (!decision.allowed) expect(decision.id).toBeNull();
    }
  });

  it('reports the request id on refusals so the module is not left waiting', () => {
    const decision = decideModuleCall(call({ path: '/api/agents' }), REQUIRES);
    expect(decision.allowed).toBe(false);
    if (!decision.allowed) expect(decision.id).toBe('r1');
  });

  it('refuses a call with no id, because a refusal could not be delivered', () => {
    const decision = decideModuleCall(call({ id: '' }), REQUIRES);
    expect(decision.allowed).toBe(false);
    if (!decision.allowed) expect(decision.id).toBeNull();
  });

  it('matches the method case-insensitively but the path exactly', () => {
    expect(decideModuleCall(call({ method: 'get' }), REQUIRES).allowed).toBe(true);
    // A trailing slash is a different path to the router, so it is one here too.
    expect(decideModuleCall(call({ path: '/api/system/health/' }), REQUIRES).allowed).toBe(false);
  });

  it('grants nothing when the manifest declared nothing', () => {
    expect(decideModuleCall(call(), []).allowed).toBe(false);
  });
});

describe('decideModuleCall with a segment wildcard', () => {
  const WILDCARD = ['GET /api/published/*'];
  const at = (path: string) => decideModuleCall(call({ path }), WILDCARD).allowed;

  it('admits one further segment, which is the point of declaring it', () => {
    expect(at('/api/published/cil')).toBe(true);
    expect(at('/api/published/anything-else_9')).toBe(true);
  });

  it('admits nothing beyond that segment', () => {
    // Two segments is a different route, and the manifest named one.
    expect(at('/api/published/cil/history')).toBe(false);
    // The bare route is not a row of it; declaring rows does not declare the list.
    expect(at('/api/published')).toBe(false);
    expect(at('/api/published/')).toBe(false);
  });

  it('does not let the segment mean somewhere else', () => {
    // The case the wildcard would otherwise open: a traversal reads as a
    // publisher name in the manifest and resolves to another route entirely.
    expect(at('/api/published/../agents')).toBe(false);
    expect(at('/api/published/..')).toBe(false);
    expect(at('/api/published/.')).toBe(false);
    // A query or fragment ends the path and starts something the manifest
    // reader never saw.
    expect(at('/api/published/cil?as=agents')).toBe(false);
    expect(at('/api/published/cil#x')).toBe(false);
  });

  it('is still bound by the route and the method the manifest named', () => {
    expect(at('/api/agents/cil')).toBe(false);
    expect(decideModuleCall(call({ method: 'POST', path: '/api/published/cil' }), WILDCARD).allowed).toBe(false);
  });

  it('leaves a star anywhere but the end as an ordinary character', () => {
    // Only a trailing `/*` is a wildcard. Anything else is matched literally,
    // so a manifest cannot reach a route by putting a star in the middle of it.
    expect(decideModuleCall(call({ path: '/api/published/cil/x' }), ['GET /api/*/cil/x']).allowed).toBe(false);
    expect(decideModuleCall(call({ path: '/api/pub*shed' }), ['GET /api/pub*shed']).allowed).toBe(true);
  });
});
