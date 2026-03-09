/**
 * Standalone VRM viewer page for separate window mode.
 * Opened via /vrm-viewer/:agentId route.
 * Self-contained: manages its own SSE subscription and VRM context.
 */
import { useEffect, useRef, useState } from 'react';
import { useParams } from 'react-router-dom';
import * as THREE from 'three';
import { Activity, AlertTriangle } from 'lucide-react';
import { VrmSceneManager } from './engine/VrmSceneManager';
import { VrmModelLoader } from './engine/VrmModelLoader';
import { VrmAnimationController } from './engine/VrmAnimationController';
import { useEventStream } from '../hooks/useEventStream';
import { EVENTS_URL, api } from '../services/api';
import { AvatarAgentState } from './engine/types';
import { isTauri } from '../lib/tauri';

export function VrmViewerPage() {
  const { agentId } = useParams<{ agentId: string }>();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const controllerRef = useRef<VrmAnimationController | null>(null);

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [agentState, setAgentState] = useState<AvatarAgentState>('idle');
  const idleTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Read API key from URL search params (passed by opener)
  const apiKey = new URLSearchParams(window.location.search).get('key') || undefined;

  // SSE subscription for agent state
  useEventStream(EVENTS_URL, (event) => {
    if (!event.data || event.data.agent_id !== agentId) return;

    switch (event.type) {
      case 'AgentThinking':
        clearIdleTimeout();
        setAgentState('thinking');
        break;
      case 'ThoughtResponse':
        clearIdleTimeout();
        setAgentState('responding');
        idleTimeoutRef.current = setTimeout(() => setAgentState('idle'), 3000);
        break;
      case 'AgenticLoopCompleted':
        clearIdleTimeout();
        setAgentState('idle');
        break;
    }
  }, apiKey);

  function clearIdleTimeout() {
    if (idleTimeoutRef.current) {
      clearTimeout(idleTimeoutRef.current);
      idleTimeoutRef.current = null;
    }
  }

  useEffect(() => () => clearIdleTimeout(), []);

  // Make html/body fully transparent for this window
  useEffect(() => {
    const prevHtml = document.documentElement.style.background;
    const prevBody = document.body.style.background;
    document.documentElement.style.background = 'transparent';
    document.body.style.background = 'transparent';
    return () => {
      document.documentElement.style.background = prevHtml;
      document.body.style.background = prevBody;
    };
  }, []);

  // Sync agent state to controller
  useEffect(() => {
    controllerRef.current?.setAgentState(agentState);
  }, [agentState]);

  // Initialize scene and load VRM
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !agentId) return;

    let disposed = false;

    const scene = new VrmSceneManager(canvas);
    const loader = new VrmModelLoader(scene.scene);
    const controller = new VrmAnimationController(scene);
    controllerRef.current = controller;

    setLoading(true);
    setError(null);

    const vrmUrl = `${api.getAvatarUrl(agentId).replace('/avatar', '/vrm')}`;

    loader.load(vrmUrl)
      .then((vrm) => {
        if (disposed) { loader.dispose(); return; }
        controller.setVrm(vrm);

        // Frame camera on head bone for centered face view
        const headBone = vrm.humanoid?.getRawBoneNode('head');
        if (headBone) {
          headBone.updateWorldMatrix(true, false);
          const headPos = new THREE.Vector3();
          headBone.getWorldPosition(headPos);
          scene.frameHead(headPos.y);
        }

        controller.start();
        setLoading(false);
      })
      .catch((err) => {
        if (disposed) return;
        console.error('VRM load failed:', err);
        setError(err.message || 'Failed to load VRM model');
        setLoading(false);
      });

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
      controllerRef.current = null;
    };
  }, [agentId]);

  // Enable window dragging in Tauri (decorations: false)
  useEffect(() => {
    if (!isTauri) return;
    const handleDrag = async (e: MouseEvent) => {
      // Only drag on the background (canvas), not on UI overlays
      if ((e.target as HTMLElement).tagName === 'CANVAS') {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        await getCurrentWindow().startDragging();
      }
    };
    document.addEventListener('mousedown', handleDrag);
    return () => document.removeEventListener('mousedown', handleDrag);
  }, []);

  return (
    <div className="w-screen h-screen bg-transparent overflow-hidden relative">
      <canvas
        ref={canvasRef}
        className="w-full h-full"
        style={{ display: loading || error ? 'none' : 'block' }}
      />

      {loading && (
        <div className="absolute inset-0 flex flex-col items-center justify-center text-white/60">
          <Activity size={24} className="animate-pulse" />
          <p className="text-[10px] font-mono tracking-[0.2em] uppercase mt-2">Loading VRM...</p>
        </div>
      )}

      {error && (
        <div className="absolute inset-0 flex flex-col items-center justify-center text-white/60 p-4">
          <AlertTriangle size={20} className="text-amber-500/60 mb-2" />
          <p className="text-[10px] font-mono text-center leading-relaxed">{error}</p>
        </div>
      )}

      {/* Agent state indicator */}
      {!loading && !error && agentState !== 'idle' && (
        <div className="absolute bottom-3 left-3 px-2.5 py-1 rounded-full text-[10px] font-mono uppercase tracking-widest bg-black/50 border border-white/10 text-white/80 backdrop-blur-sm">
          <span className={`inline-block w-1.5 h-1.5 rounded-full mr-1.5 ${
            agentState === 'thinking' ? 'bg-blue-400 animate-pulse' : 'bg-brand animate-pulse'
          }`} />
          {agentState}
        </div>
      )}
    </div>
  );
}
