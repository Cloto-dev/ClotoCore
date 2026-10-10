import { type RefObject, useEffect, useRef, useState } from 'react';
import { useApi } from '../hooks/useApi';
import { useEventStream } from '../hooks/useEventStream';
import { EVENTS_URL, api as rawApi } from '../services/api';
import { browserSessionReady } from '../services/session';
import { applyAvatarEvent } from './avatarEvents';
import type { AvatarAgentState } from './engine/types';
import { VrmAnimationController } from './engine/VrmAnimationController';
import { VrmModelLoader } from './engine/VrmModelLoader';
import { VrmSceneManager } from './engine/VrmSceneManager';

export function useVrmRuntime(
  canvas: RefObject<HTMLCanvasElement>,
  url: string | null,
  agentId: string,
  active = true,
  framing: 'body' | 'head' = 'body',
) {
  const api = useApi();
  const activeRef = useRef(active);
  activeRef.current = active;
  const controller = useRef<VrmAnimationController | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [agentState, setAgentState] = useState<AvatarAgentState>('idle');
  const idleTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const eventEpoch = useRef(0);
  useEventStream(
    EVENTS_URL,
    (event) => {
      if (!active || !controller.current) return;
      applyAvatarEvent(event, agentId, controller.current, (state) => {
        eventEpoch.current++;
        if (idleTimer.current) clearTimeout(idleTimer.current);
        setAgentState(state);
        controller.current?.setAgentState(state);
        if (state === 'responding')
          idleTimer.current = setTimeout(() => {
            setAgentState('idle');
            controller.current?.setAgentState('idle');
          }, 3000);
      });
      if (
        event.type === 'ThoughtResponse' &&
        event.data.agent_id === agentId &&
        event.data.content &&
        !event.data.auto_spoken
      ) {
        const epoch = eventEpoch.current;
        const target = controller.current;
        rawApi
          .generateVisemes(agentId, String(event.data.content), api.apiKey)
          .then((timeline) => {
            if (target === controller.current && epoch === eventEpoch.current) target.playVisemes(timeline.entries);
          })
          .catch(() => {});
      }
    },
    api.apiKey,
  );
  useEffect(() => {
    const element = canvas.current;
    if (!element || !url) return;
    let disposed = false;
    let scene: VrmSceneManager | null = null;
    let loader: VrmModelLoader | null = null;
    let animation: VrmAnimationController | null = null;
    setLoading(true);
    setError(null);
    setAgentState('idle');
    const lost = (e: Event) => {
      e.preventDefault();
      animation?.stop();
      setError('WebGL');
    };
    element.addEventListener('webglcontextlost', lost);
    browserSessionReady()
      .then(async () => {
        if (disposed) return;
        scene = new VrmSceneManager(element);
        loader = new VrmModelLoader(scene.scene);
        animation = new VrmAnimationController(scene);
        controller.current = animation;
        const vrm = await loader.load(url);
        if (disposed) {
          loader.dispose();
          return;
        }
        animation.setVrm(vrm);
        if (framing === 'body') scene.frameBody(vrm.scene);
        else {
          const head = vrm.humanoid?.getRawBoneNode('head');
          if (head) {
            head.updateWorldMatrix(true, false);
            scene.frameHead(head.matrixWorld.elements[13]);
          }
        }
        if (activeRef.current) animation.start();
        setLoading(false);
      })
      .catch((err) => {
        if (!disposed) {
          setLoading(false);
          setError(String(err.message ?? err));
        }
      });
    return () => {
      disposed = true;
      eventEpoch.current++;
      if (idleTimer.current) clearTimeout(idleTimer.current);
      element.removeEventListener('webglcontextlost', lost);
      animation?.dispose();
      loader?.dispose();
      scene?.dispose();
      controller.current = null;
    };
  }, [canvas, url, framing]);
  useEffect(() => {
    if (active) controller.current?.start();
    else controller.current?.stop();
  }, [active]);
  return { controller, loading, error, agentState };
}
