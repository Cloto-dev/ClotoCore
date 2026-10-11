import { afterEach, describe, expect, it, vi } from 'vitest';

/**
 * `isTauri` is decided once, when `lib/tauri` is first imported, so the only way
 * to test both shells is to re-import the module graph under each one.
 */
async function loadApi(shell: 'browser' | 'tauri') {
  vi.resetModules();
  if (shell === 'tauri') {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  } else {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  }
  return await import('../api');
}

afterEach(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  vi.resetModules();
});

// The browser fetches avatars, VRM models and chat attachments by URL, so the
// admin key cannot ride in a header. Which credential fills that gap depends on
// the shell, and these assertions pin both halves — the browser's URLs must be
// clean because the session cookie covers them, and Tauri's must still carry the
// key because its origin is not the API's and no cookie is ever sent.
describe('asset URL builders in a browser', () => {
  it('carry no credential at all', async () => {
    const { API_BASE, api } = await loadApi('browser');
    expect(api.getAvatarUrl('agent-1', 'k1')).toBe(`${API_BASE}/agents/agent-1/avatar`);
    expect(api.getVrmUrl('agent-1', 'k1')).toBe(`${API_BASE}/agents/agent-1/vrm`);
    expect(api.getAttachmentUrl('att-1', 'k1')).toBe(`${API_BASE}/chat/attachments/att-1`);
  });

  it('still append the other query params, with "?" first', async () => {
    const { API_BASE, api } = await loadApi('browser');
    expect(api.getAvatarUrl('agent-1', 'k1', 42)).toBe(`${API_BASE}/agents/agent-1/avatar?v=42`);
    expect(api.getAvatarUrl('agent-1', '', 42)).toBe(`${API_BASE}/agents/agent-1/avatar?v=42`);
  });

  /**
   * The point of the whole change, stated as one property rather than as a
   * per-builder expectation: whatever a caller passes, the key does not reach
   * the address bar, the history entry, the `Referer`, or a proxy log.
   */
  it('never put the key in a URL, whichever builder or binding is used', async () => {
    const { api, createAuthenticatedApi } = await loadApi('browser');
    const key = 'a-secret-admin-key';
    const bound = createAuthenticatedApi(key);
    const urls = [
      api.getAvatarUrl('agent-1', key),
      api.getAvatarUrl('agent-1', key, 42),
      api.getVrmUrl('agent-1', key),
      api.getAttachmentUrl('att-1', key),
      bound.getAvatarUrl('agent-1'),
      bound.getAvatarUrl('agent-1', 7),
      bound.getVrmUrl('agent-1'),
      bound.getAttachmentUrl('att-1'),
    ];
    for (const url of urls) {
      expect(url, url).not.toContain(key);
      expect(url, url).not.toContain('token=');
    }
  });
});

describe('asset URL builders under Tauri', () => {
  it('append the token with "?" when the URL has no query yet', async () => {
    const { API_BASE, api } = await loadApi('tauri');
    expect(api.getAvatarUrl('agent-1', 'k1')).toBe(`${API_BASE}/agents/agent-1/avatar?token=k1`);
    expect(api.getVrmUrl('agent-1', 'k1')).toBe(`${API_BASE}/agents/agent-1/vrm?token=k1`);
    expect(api.getAttachmentUrl('att-1', 'k1')).toBe(`${API_BASE}/chat/attachments/att-1?token=k1`);
  });

  it('append the cache-buster with "&" after the token', async () => {
    const { API_BASE, api } = await loadApi('tauri');
    expect(api.getAvatarUrl('agent-1', 'k1', 42)).toBe(`${API_BASE}/agents/agent-1/avatar?token=k1&v=42`);
  });

  it('keep the URL valid (no dangling "token=") when the key is empty', async () => {
    const { API_BASE, api } = await loadApi('tauri');
    expect(api.getAvatarUrl('agent-1', '')).toBe(`${API_BASE}/agents/agent-1/avatar`);
    expect(api.getAvatarUrl('agent-1', '', 42)).toBe(`${API_BASE}/agents/agent-1/avatar?v=42`);
    expect(api.getVrmUrl('agent-1', '')).not.toContain('token=');
    expect(api.getAttachmentUrl('att-1', '')).not.toContain('token=');
  });

  it('percent-encode a key with query-significant characters', async () => {
    const { API_BASE, api } = await loadApi('tauri');
    expect(api.getAvatarUrl('agent-1', 'a&b c')).toBe(`${API_BASE}/agents/agent-1/avatar?token=a%26b%20c`);
  });

  it('bind the key through createAuthenticatedApi', async () => {
    const { API_BASE, createAuthenticatedApi } = await loadApi('tauri');
    const authed = createAuthenticatedApi('k2');
    expect(authed.getAvatarUrl('agent-1')).toBe(`${API_BASE}/agents/agent-1/avatar?token=k2`);
    expect(authed.getAvatarUrl('agent-1', 7)).toBe(`${API_BASE}/agents/agent-1/avatar?token=k2&v=7`);
    expect(authed.getVrmUrl('agent-1')).toBe(`${API_BASE}/agents/agent-1/vrm?token=k2`);
    expect(authed.getAttachmentUrl('att-1')).toBe(`${API_BASE}/chat/attachments/att-1?token=k2`);
  });
});

// A screen decides between "not allowed" and "not there" by the status, so the
// status has to survive the throw; the message alone is prose from the kernel.
describe('a refused request', () => {
  it('throws with the HTTP status and the kernel message', async () => {
    const { api } = await loadApi('browser');
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValue(
          new Response(JSON.stringify({ error: { message: 'Permission denied: AdminAccess' } }), { status: 403 }),
        ),
    );
    await expect(api.getAgents('')).rejects.toMatchObject({ status: 403, message: 'Permission denied: AdminAccess' });
    vi.unstubAllGlobals();
  });
});

// The settings screen shows the memory server the kernel itself picks for the
// agent; the answer has to come from the agent's own route, with the key.
describe("an agent's memory target", () => {
  it("is read from the agent's memory route with the key", async () => {
    const { createAuthenticatedApi } = await loadApi('browser');
    const target = { plugin: null, server_id: 'cpersona', allowed: true };
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ data: target }), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);
    await expect(createAuthenticatedApi('k1').getAgentMemory('agent.mio/x')).resolves.toEqual(target);
    const [url, init] = fetchMock.mock.calls[0];
    expect(String(url)).toMatch(/\/agents\/agent\.mio%2Fx\/memory$/);
    expect(init.headers['X-API-Key']).toBe('k1');
    vi.unstubAllGlobals();
  });
});
