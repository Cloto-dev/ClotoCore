import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata } from '../types';
import { partnerMediaChanged, usePartnerMedia } from './usePartnerMedia';

const m = vi.hoisted(() => ({
  getVrmUrl: vi.fn((id: string) => `/vrm/${id}`),
  getAvatarUrl: vi.fn((id: string) => `/icon/${id}`),
  extractVrmThumbnail: vi.fn(),
}));
vi.mock('../hooks/useApi', () => ({ useApi: () => m }));
vi.mock('../services/session', () => ({ browserSessionReady: () => Promise.resolve() }));
vi.mock('../lib/vrmThumbnail', () => ({ extractVrmThumbnail: m.extractVrmThumbnail }));
beforeEach(() => {
  m.extractVrmThumbnail.mockReset();
  m.extractVrmThumbnail.mockResolvedValue(new File(['thumbnail'], 'thumbnail.png', { type: 'image/png' }));
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, blob: () => Promise.resolve(new Blob(['vrm'])) }));
});
describe('partner icon priority', () => {
  it('prefers a manual image and restores the current embedded thumbnail when it is removed', async () => {
    const a = {
      id: 'agent.manual',
      name: 'Mio',
      description: '',
      required_capabilities: [],
      enabled: true,
      last_seen: 0,
      status: 'online',
      metadata: { has_vrm: 'true', has_avatar: 'true' },
    } as AgentMetadata;
    const { result, rerender } = renderHook(({ agent }) => usePartnerMedia(agent), { initialProps: { agent: a } });
    expect(result.current.iconUrl).toBe('/icon/agent.manual');
    await waitFor(() => expect(m.extractVrmThumbnail).toHaveBeenCalled());
    rerender({ agent: { ...a, metadata: { has_vrm: 'true' } } });
    await waitFor(() => expect(result.current.iconUrl).toMatch(/^data:image\/png;base64,/));
    rerender({ agent: { ...a, metadata: {} } });
    expect(result.current.iconUrl).toBeNull();
    expect(result.current.vrmUrl).toBeNull();
  });
  it('changes the model URL on replacement and never retains the previous thumbnail', async () => {
    const a = {
      id: 'agent.replace',
      name: 'Mio',
      description: '',
      required_capabilities: [],
      enabled: true,
      last_seen: 0,
      status: 'online',
      metadata: { has_vrm: 'true' },
    } as AgentMetadata;
    const { result } = renderHook(() => usePartnerMedia(a));
    await waitFor(() => expect(result.current.iconUrl).toMatch(/^data:/));
    const previous = result.current.vrmUrl;
    m.extractVrmThumbnail.mockResolvedValue(null);
    act(() => partnerMediaChanged(a.id));
    expect(result.current.vrmUrl).not.toBe(previous);
    expect(result.current.iconUrl).toBeNull();
    await waitFor(() => expect(m.extractVrmThumbnail).toHaveBeenCalledTimes(2));
    expect(result.current.iconUrl).toBeNull();
  });
});
