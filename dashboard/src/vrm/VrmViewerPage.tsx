import { Settings, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';
import { useAgentContext } from '../contexts/AgentContext';
import { AgentIcon } from '../lib/agentIdentity';
import { isTauri } from '../lib/tauri';
import defaultPartner from '../mate/assets/default-partner.svg';
import { usePartnerMedia } from '../mate/usePartnerMedia';
import { MotionControls } from './MotionControls';
import { applySavedMotion } from './motionSettings';
import { useVrmRuntime } from './useVrmRuntime';
import '../mate/MizMate.css';
import './VrmViewer.css';

/** Desktop and browser viewer share the room's model, event bridge and motion controls. */
export function VrmViewerPage() {
  const { agentId = '' } = useParams<{ agentId: string }>();
  const { agents, isLoading, refetchAgents } = useAgentContext();
  const agent = agents.find((a) => a.id === agentId) ?? null;
  const { t } = useTranslation('agents');
  const canvas = useRef<HTMLCanvasElement>(null);
  const { iconUrl, vrmUrl } = usePartnerMedia(agent);
  const runtime = useVrmRuntime(canvas, vrmUrl, agentId, true, 'body');
  const [settings, setSettings] = useState(false);
  const ready = Boolean(vrmUrl && !runtime.loading && !runtime.error && runtime.controller.current);
  useEffect(() => {
    if (agent && ready) {
      if (runtime.controller.current) applySavedMotion(agent, runtime.controller.current);
    }
  }, [
    agent?.id,
    agent?.metadata?.mate_idle,
    agent?.metadata?.mate_pose,
    agent?.metadata?.mate_motion_style,
    agent?.metadata?.mate_companion_idle,
    agent?.metadata?.mate_companion_pose,
    ready,
    runtime.controller,
  ]);
  useEffect(() => {
    const html = document.documentElement.style.background;
    const body = document.body.style.background;
    document.documentElement.style.background = 'transparent';
    document.body.style.background = 'transparent';
    return () => {
      document.documentElement.style.background = html;
      document.body.style.background = body;
    };
  }, []);
  return (
    <div className="mizmate mate-desktop-viewer">
      <div
        className="mate-desktop-stage"
        onMouseDown={async (e) => {
          if (isTauri && e.button === 0 && e.clientY < window.innerHeight / 2) {
            const { getCurrentWindow } = await import('@tauri-apps/api/window');
            await getCurrentWindow().startDragging();
          }
        }}
      >
        {!agent && (
          <p role={isLoading ? 'status' : 'alert'}>
            {t(isLoading ? 'mate.loading_partner' : 'mate.partner_unavailable')}
          </p>
        )}
        <canvas
          ref={canvas}
          hidden={!vrmUrl || Boolean(runtime.error)}
          style={{ visibility: runtime.loading ? 'hidden' : undefined }}
          aria-label={t('mate.vrm_alt', { name: agent?.name ?? '' })}
        />
        {!vrmUrl && agent && (
          <img
            className={iconUrl ? 'mate-image-avatar' : 'mate-default-avatar'}
            src={iconUrl ?? defaultPartner}
            alt={agent.name}
          />
        )}
        {runtime.loading && <p role="status">{t('mate.loading_vrm')}</p>}
        {runtime.error && <p role="alert">{t('mate.vrm_failed')}</p>}
      </div>
      {agent && (
        <div className="mate-desktop-toolbar">
          <span>
            <AgentIcon agent={agent} size={24} />
            {agent.name}
          </span>
          <button type="button" aria-expanded={settings} onClick={() => setSettings(!settings)}>
            <Settings size={16} />
            {t('mate.motion')}
          </button>
        </div>
      )}
      {settings && agent && (
        <div className="mate-desktop-settings">
          <button
            type="button"
            className="mate-desktop-close"
            aria-label={t('mate.close_settings')}
            onClick={() => setSettings(false)}
          >
            <X size={18} />
          </button>
          <MotionControls agent={agent} controller={runtime.controller} ready={ready} onSaved={refetchAgents} />
        </div>
      )}
    </div>
  );
}
