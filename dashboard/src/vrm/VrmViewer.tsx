import { Activity, AlertTriangle } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { api } from '../services/api';
import { VrmAnimationController } from './engine/VrmAnimationController';
import { VrmModelLoader } from './engine/VrmModelLoader';
import { VrmSceneManager } from './engine/VrmSceneManager';
import { useVrmAvatar } from './useVrmAvatar';
import { useVrmContext } from './VrmContext';

interface Props {
  agentId: string;
  apiKey?: string;
}

export function VrmViewer({ agentId, apiKey }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const controllerRef = useRef<VrmAnimationController | null>(null);
  const sceneRef = useRef<VrmSceneManager | null>(null);
  const loaderRef = useRef<VrmModelLoader | null>(null);
  const { agentState, idleParams, vrmVisible } = useVrmContext();

  // SSE → agent state → POSE_THINKING / responding / idle auto-transition
  useVrmAvatar(agentId, apiKey);

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Initialize three.js scene and load VRM
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !vrmVisible) return;

    let disposed = false;

    const scene = new VrmSceneManager(canvas);
    const loader = new VrmModelLoader(scene.scene);
    const controller = new VrmAnimationController(scene);

    sceneRef.current = scene;
    loaderRef.current = loader;
    controllerRef.current = controller;

    setLoading(true);
    setError(null);

    const vrmUrl = `${api.getAvatarUrl(agentId).replace('/avatar', '/vrm')}`;

    loader
      .load(vrmUrl)
      .then((vrm) => {
        if (disposed) {
          loader.dispose();
          return;
        }
        controller.setVrm(vrm);
        controller.start();
        setLoading(false);
      })
      .catch((err) => {
        if (disposed) return;
        console.error('VRM load failed:', err);
        setError(err.message || 'Failed to load VRM model');
        setLoading(false);
      });

    // WebGL context lost handler
    const handleContextLost = (e: Event) => {
      e.preventDefault();
      setError('WebGL context lost');
      controller.stop();
    };
    canvas.addEventListener('webglcontextlost', handleContextLost);

    return () => {
      disposed = true;
      canvas.removeEventListener('webglcontextlost', handleContextLost);
      controller.dispose();
      loader.dispose();
      scene.dispose();
      sceneRef.current = null;
      loaderRef.current = null;
      controllerRef.current = null;
    };
  }, [agentId, vrmVisible]);

  // Sync agent state changes to controller
  useEffect(() => {
    controllerRef.current?.setAgentState(agentState);
  }, [agentState]);

  // Sync idle params to controller
  useEffect(() => {
    controllerRef.current?.setIdleParams(idleParams);
  }, [idleParams]);

  // Stop/start animation when visibility toggles
  useEffect(() => {
    if (vrmVisible) {
      controllerRef.current?.start();
    } else {
      controllerRef.current?.stop();
    }
  }, [vrmVisible]);

  if (!vrmVisible) return null;

  return (
    <div className="relative w-64 h-full border-r border-edge bg-glass-strong flex-shrink-0">
      <canvas ref={canvasRef} className="w-full h-full" style={{ display: loading || error ? 'none' : 'block' }} />

      {loading && (
        <div className="absolute inset-0 flex flex-col items-center justify-center text-content-tertiary">
          <Activity size={24} className="animate-pulse" />
          <p className="text-[10px] font-mono tracking-[0.2em] uppercase mt-2">Loading VRM...</p>
        </div>
      )}

      {error && (
        <div className="absolute inset-0 flex flex-col items-center justify-center text-content-tertiary p-4">
          <AlertTriangle size={20} className="text-amber-500/60 mb-2" />
          <p className="text-[10px] font-mono text-center leading-relaxed">{error}</p>
        </div>
      )}

      {/* Agent state indicator */}
      {!loading && !error && agentState !== 'idle' && (
        <div className="absolute bottom-2 left-2 px-2 py-0.5 rounded-full text-[9px] font-mono uppercase tracking-widest bg-glass border border-edge">
          <span
            className={`inline-block w-1.5 h-1.5 rounded-full mr-1 ${
              agentState === 'thinking' ? 'bg-blue-400 animate-pulse' : 'bg-brand animate-pulse'
            }`}
          />
          {agentState}
        </div>
      )}
    </div>
  );
}
