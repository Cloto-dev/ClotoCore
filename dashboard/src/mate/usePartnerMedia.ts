import { useEffect, useState } from 'react';
import { useApi } from '../hooks/useApi';
import { extractVrmThumbnail } from '../lib/vrmThumbnail';
import { browserSessionReady } from '../services/session';
import type { AgentMetadata } from '../types';

export const PARTNER_MEDIA_CHANGED = 'mizmate-partner-media-changed';
const versions = new Map<string, number>();
const thumbnails = new Map<string, Promise<string | null>>();
export const PARTNER_SETTINGS_CHANGED = 'mizmate-partner-settings-changed';
const channel = typeof window.BroadcastChannel === 'function' ? new BroadcastChannel('mizmate-partner-settings') : null;
function notify(agentId: string, media: boolean) {
  if (media) {
    versions.set(agentId, (versions.get(agentId) ?? 0) + 1);
    thumbnails.clear();
    window.dispatchEvent(new CustomEvent(PARTNER_MEDIA_CHANGED, { detail: agentId }));
  }
  window.dispatchEvent(new CustomEvent(PARTNER_SETTINGS_CHANGED, { detail: agentId }));
}
if (channel)
  channel.onmessage = (event) => {
    if (typeof event.data?.agentId === 'string') notify(event.data.agentId, event.data.media === true);
  };
export function partnerMediaChanged(agentId: string) {
  notify(agentId, true);
  channel?.postMessage({ agentId, media: true });
}
export function partnerSettingsChanged(agentId: string) {
  notify(agentId, false);
  channel?.postMessage({ agentId, media: false });
}

function dataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

/** Derive the automatic icon from the current saved VRM, without overwriting a manual image. */
export function usePartnerMedia(agent: AgentMetadata | null) {
  const api = useApi();
  const id = agent?.id ?? '';
  const [revision, setRevision] = useState(versions.get(id) ?? 0);
  const [thumbnail, setThumbnail] = useState<{ url: string; source: string } | null>(null);
  useEffect(() => {
    const refresh = (e: Event) => {
      if ((e as CustomEvent).detail === id) setRevision(versions.get(id) ?? 0);
    };
    window.addEventListener(PARTNER_MEDIA_CHANGED, refresh);
    setRevision(versions.get(id) ?? 0);
    return () => window.removeEventListener(PARTNER_MEDIA_CHANGED, refresh);
  }, [id]);
  const hasVrm = agent?.metadata?.has_vrm === 'true';
  const version = `${agent?.metadata?.vrm_path ?? ''}:${revision}`;
  const raw = hasVrm ? api.getVrmUrl(id) : '';
  const vrmUrl = hasVrm ? `${raw}${raw.includes('?') ? '&' : '?'}v=${encodeURIComponent(version)}` : null;
  useEffect(() => {
    if (!vrmUrl) {
      setThumbnail(null);
      return;
    }
    let cancelled = false;
    let pending = thumbnails.get(vrmUrl);
    if (!pending) {
      pending = browserSessionReady().then(async () => {
        const res = await fetch(vrmUrl, { cache: 'no-store' });
        if (!res.ok) throw new Error('thumbnail unavailable');
        const file = new File([await res.blob()], 'partner.vrm');
        const thumb = await extractVrmThumbnail(file);
        return thumb ? dataUrl(thumb) : null;
      });
      thumbnails.set(vrmUrl, pending);
      pending.catch(() => thumbnails.delete(vrmUrl));
    }
    pending
      .then((url) => {
        if (!cancelled) setThumbnail(url ? { url, source: vrmUrl } : null);
      })
      .catch(() => {
        if (!cancelled) setThumbnail(null);
      });
    return () => {
      cancelled = true;
    };
  }, [vrmUrl]);
  const automatic = thumbnail?.source === vrmUrl ? thumbnail.url : null;
  const iconUrl =
    agent?.metadata?.has_avatar === 'true'
      ? api.getAvatarUrl(
          id,
          revision ? `${agent.metadata.avatar_updated_at ?? ''}:${revision}` : agent.metadata.avatar_updated_at,
        )
      : automatic;
  return { iconUrl, vrmUrl };
}
