import { describe, expect, it } from 'vitest';
import { API_BASE, api, createAuthenticatedApi } from '../api';

// The browser fetches avatars, VRM models and chat attachments by URL, so the
// admin key cannot ride in a header — the kernel takes it as `token=` on those
// read routes. These assertions pin the query-string shape the kernel parses.
describe('asset URL builders carry the admin key as a query param', () => {
  it('appends the token with "?" when the URL has no query yet', () => {
    expect(api.getAvatarUrl('agent-1', 'k1')).toBe(`${API_BASE}/agents/agent-1/avatar?token=k1`);
    expect(api.getVrmUrl('agent-1', 'k1')).toBe(`${API_BASE}/agents/agent-1/vrm?token=k1`);
    expect(api.getAttachmentUrl('att-1', 'k1')).toBe(`${API_BASE}/chat/attachments/att-1?token=k1`);
  });

  it('appends the cache-buster with "&" after the token', () => {
    expect(api.getAvatarUrl('agent-1', 'k1', 42)).toBe(`${API_BASE}/agents/agent-1/avatar?token=k1&v=42`);
  });

  it('keeps the URL valid (no dangling "token=") when the key is empty', () => {
    expect(api.getAvatarUrl('agent-1', '')).toBe(`${API_BASE}/agents/agent-1/avatar`);
    expect(api.getAvatarUrl('agent-1', '', 42)).toBe(`${API_BASE}/agents/agent-1/avatar?v=42`);
    expect(api.getVrmUrl('agent-1', '')).not.toContain('token=');
    expect(api.getAttachmentUrl('att-1', '')).not.toContain('token=');
  });

  it('percent-encodes a key with query-significant characters', () => {
    expect(api.getAvatarUrl('agent-1', 'a&b c')).toBe(`${API_BASE}/agents/agent-1/avatar?token=a%26b%20c`);
  });

  it('binds the key through createAuthenticatedApi', () => {
    const authed = createAuthenticatedApi('k2');
    expect(authed.getAvatarUrl('agent-1')).toBe(`${API_BASE}/agents/agent-1/avatar?token=k2`);
    expect(authed.getAvatarUrl('agent-1', 7)).toBe(`${API_BASE}/agents/agent-1/avatar?token=k2&v=7`);
    expect(authed.getVrmUrl('agent-1')).toBe(`${API_BASE}/agents/agent-1/vrm?token=k2`);
    expect(authed.getAttachmentUrl('att-1')).toBe(`${API_BASE}/chat/attachments/att-1?token=k2`);
  });
});
