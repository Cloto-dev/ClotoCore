import { type RefObject, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import type { AgentMetadata } from '../types';
import type { VrmAnimationController } from '../vrm/engine/VrmAnimationController';
import { applySavedMotion } from '../vrm/motionSettings';
import { useVrmRuntime } from '../vrm/useVrmRuntime';
import defaultPartner from './assets/default-partner.svg';
import { usePartnerMedia } from './usePartnerMedia';

export function PartnerStage({
  agent,
  active,
  onRuntime,
}: {
  agent: AgentMetadata;
  active: boolean;
  onRuntime: (controller: RefObject<VrmAnimationController | null>, ready: boolean) => void;
}) {
  const { t } = useTranslation('agents');
  const canvas = useRef<HTMLCanvasElement>(null);
  const { iconUrl, vrmUrl } = usePartnerMedia(agent);
  const runtime = useVrmRuntime(canvas, vrmUrl, agent.id, active);
  const ready = Boolean(vrmUrl && !runtime.loading && !runtime.error && runtime.controller.current);
  useEffect(() => {
    onRuntime(runtime.controller, ready);
  }, [onRuntime, runtime.controller, ready]);
  useEffect(() => {
    if (ready) {
      if (runtime.controller.current) applySavedMotion(agent, runtime.controller.current);
    }
  }, [
    ready,
    runtime.controller,
    agent.id,
    agent.metadata?.mate_idle,
    agent.metadata?.mate_pose,
    agent.metadata?.mate_motion_style,
    agent.metadata?.mate_companion_idle,
    agent.metadata?.mate_companion_pose,
  ]);
  return (
    <div className="mate-portrait" data-appearance={vrmUrl ? 'vrm' : iconUrl ? 'image' : 'default'}>
      <canvas
        ref={canvas}
        hidden={!vrmUrl || Boolean(runtime.error)}
        style={{ visibility: runtime.loading ? 'hidden' : undefined }}
        aria-label={t('mate.vrm_alt', { name: agent.name })}
      />
      {!vrmUrl &&
        (iconUrl ? (
          <img className="mate-image-avatar" src={iconUrl} alt={agent.name} />
        ) : (
          <img className="mate-default-avatar" src={defaultPartner} alt={t('mate.default_alt')} />
        ))}
      {vrmUrl && runtime.loading && <p role="status">{t('mate.loading_vrm')}</p>}
      {vrmUrl && runtime.error && (
        <p role="alert" className="mate-problem">
          {t('mate.vrm_failed')}
        </p>
      )}
    </div>
  );
}
