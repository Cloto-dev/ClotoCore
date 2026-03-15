/**
 * Standalone VRM viewer page for separate window mode.
 * Opened via /vrm-viewer/:agentId route.
 * Self-contained: manages its own SSE subscription and VRM context.
 */

import { Activity, AlertTriangle, Copy, Settings, Upload } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useParams } from 'react-router-dom';
import * as THREE from 'three';
import { useEventStream } from '../hooks/useEventStream';
import { isTauri } from '../lib/tauri';
import { API_BASE, api, EVENTS_URL } from '../services/api';
import { type AvatarAgentState, DEFAULT_POSE, type DefaultPoseParams, POSE_PRESETS } from './engine/types';
import { VrmAnimationController } from './engine/VrmAnimationController';
import { VrmModelLoader } from './engine/VrmModelLoader';
import { VrmSceneManager } from './engine/VrmSceneManager';

const POSE_LABELS: Record<string, string> = {
  relaxed: 'Relaxed',
  attentive: 'Attentive',
  thinking: 'Thinking',
  arms_crossed: 'Crossed',
};

// Slider definitions grouped by body part
interface SliderDef {
  key: keyof DefaultPoseParams;
  label: string;
  min: number;
  max: number;
  step: number;
}

const SLIDER_GROUPS: { group: string; sliders: SliderDef[] }[] = [
  {
    group: 'Left Upper Arm',
    sliders: [
      { key: 'left_upper_arm_z', label: 'Z (lower)', min: -1.5, max: 2.0, step: 0.05 },
      { key: 'left_upper_arm_y', label: 'Y (fwd/back)', min: -2.0, max: 2.0, step: 0.05 },
      { key: 'left_upper_arm_x', label: 'X (twist)', min: -1.5, max: 1.5, step: 0.05 },
    ],
  },
  {
    group: 'Right Upper Arm',
    sliders: [
      { key: 'right_upper_arm_z', label: 'Z (lower)', min: -2.0, max: 1.5, step: 0.05 },
      { key: 'right_upper_arm_y', label: 'Y (fwd/back)', min: -2.0, max: 2.0, step: 0.05 },
      { key: 'right_upper_arm_x', label: 'X (twist)', min: -1.5, max: 1.5, step: 0.05 },
    ],
  },
  {
    group: 'Left Lower Arm',
    sliders: [
      { key: 'left_lower_arm_x', label: 'X (bend)', min: -1.0, max: 3.0, step: 0.05 },
      { key: 'left_lower_arm_z', label: 'Z (fold)', min: -2.0, max: 2.0, step: 0.05 },
    ],
  },
  {
    group: 'Right Lower Arm',
    sliders: [
      { key: 'right_lower_arm_x', label: 'X (bend)', min: -1.0, max: 3.0, step: 0.05 },
      { key: 'right_lower_arm_z', label: 'Z (fold)', min: -2.0, max: 2.0, step: 0.05 },
    ],
  },
  {
    group: 'Hands',
    sliders: [
      { key: 'left_hand_z', label: 'Left Z (angle)', min: -1.0, max: 1.0, step: 0.05 },
      { key: 'left_hand_x', label: 'Left X (flex)', min: -1.0, max: 1.0, step: 0.05 },
      { key: 'right_hand_z', label: 'Right Z (angle)', min: -1.0, max: 1.0, step: 0.05 },
      { key: 'right_hand_x', label: 'Right X (flex)', min: -1.0, max: 1.0, step: 0.05 },
    ],
  },
  {
    group: 'Fingers',
    sliders: [
      { key: 'finger_spread', label: 'Spread', min: 0, max: 0.3, step: 0.01 },
      { key: 'finger_curl_proximal', label: 'Curl 1st', min: 0, max: 1.5, step: 0.05 },
      { key: 'finger_curl_intermediate', label: 'Curl 2nd', min: 0, max: 1.5, step: 0.05 },
      { key: 'finger_curl_distal', label: 'Curl 3rd', min: 0, max: 1.5, step: 0.05 },
      { key: 'thumb_curl_proximal', label: 'Thumb 1st', min: 0, max: 1.0, step: 0.05 },
      { key: 'thumb_curl_distal', label: 'Thumb 2nd', min: 0, max: 1.0, step: 0.05 },
    ],
  },
  {
    group: 'Neck',
    sliders: [
      { key: 'neck_x', label: 'X (pitch)', min: -0.5, max: 0.5, step: 0.01 },
      { key: 'neck_y', label: 'Y (turn)', min: -0.5, max: 0.5, step: 0.01 },
      { key: 'neck_z', label: 'Z (tilt)', min: -0.3, max: 0.3, step: 0.01 },
    ],
  },
  {
    group: 'Spine',
    sliders: [
      { key: 'spine_x', label: 'X (lean)', min: -0.3, max: 0.3, step: 0.01 },
      { key: 'spine_y', label: 'Y (twist)', min: -0.5, max: 0.5, step: 0.01 },
      { key: 'spine_z', label: 'Z (side)', min: -0.3, max: 0.3, step: 0.01 },
    ],
  },
  {
    group: 'Head',
    sliders: [
      { key: 'head_x', label: 'X (nod)', min: -0.5, max: 0.5, step: 0.01 },
      { key: 'head_y', label: 'Y (turn)', min: -0.5, max: 0.5, step: 0.01 },
      { key: 'head_z', label: 'Z (tilt)', min: -0.3, max: 0.3, step: 0.01 },
    ],
  },
];

export function VrmViewerPage() {
  const { agentId } = useParams<{ agentId: string }>();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const controllerRef = useRef<VrmAnimationController | null>(null);

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [agentState, setAgentState] = useState<AvatarAgentState>('idle');
  const [activePose, setActivePose] = useState('relaxed');
  const [showSettings, setShowSettings] = useState(false);
  const [poseValues, setPoseValues] = useState<DefaultPoseParams>({ ...DEFAULT_POSE });
  const [copied, setCopied] = useState(false);
  const [showFooter, setShowFooter] = useState(false);
  const [vrmaName, setVrmaName] = useState<string | null>(null);
  const [isDragOver, setIsDragOver] = useState(false);
  const vrmaInputRef = useRef<HTMLInputElement>(null);
  const idleTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const footerTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Read API key from URL search params (passed by opener)
  const apiKey = new URLSearchParams(window.location.search).get('key') || undefined;

  // SSE subscription for agent state + lip sync + MGP avatar commands
  useEventStream(
    EVENTS_URL,
    (event) => {
      // MGP server notifications (output.avatar)
      if (event.type === 'McpNotification') {
        const d = event.data as Record<string, unknown>;
        const serverId = d?.server_id as string | undefined;
        const params = d?.params as Record<string, unknown> | undefined;
        if (!params) return;
        const channel = params.channel as string | undefined;
        const data = params.data as Record<string, unknown> | undefined;
        if (!data || data.agent_id !== agentId) return;

        // output.avatar server channels (includes VOICEVOX TTS)
        if (serverId === 'output.avatar') {
          if (channel === 'avatar_set_expression') {
            controllerRef.current?.setExpression(data.expression as string, (data.intensity as number) ?? 1.0);
          } else if (channel === 'avatar_set_pose') {
            controllerRef.current?.setPose(data.pose as string, (data.transition as number) ?? 0.5);
            setActivePose(data.pose as string);
          } else if (channel === 'avatar_set_idle_behavior') {
            controllerRef.current?.setIdleBehavior(data as Record<string, unknown>);
          } else if (channel === 'viseme_correction') {
            controllerRef.current?.playVisemes(
              (data.entries as Array<{ viseme: string; start_ms: number; duration_ms: number }>) ?? [],
            );
          } else if (channel === 'avatar_speech_play') {
            const timeline =
              (data.viseme_timeline as Array<{ viseme: string; start_ms: number; duration_ms: number }>) ?? [];
            const audioOffsetMs = (data.audio_offset_ms as number) ?? 0;
            if (data.audio_data) {
              controllerRef.current?.playSpeechData(data.audio_data as string, timeline, audioOffsetMs);
            } else if (data.audio_url) {
              const audioUrl = `${API_BASE}${data.audio_url as string}`;
              controllerRef.current?.playSpeech(audioUrl, timeline, audioOffsetMs);
            }
          }
        }

        return;
      }

      const evtData = event.data as Record<string, unknown> | undefined;
      if (!evtData || evtData.agent_id !== agentId) return;

      switch (event.type) {
        case 'AgentThinking':
          clearIdleTimeout();
          setAgentState('thinking');
          break;
        case 'ThoughtResponse':
          clearIdleTimeout();
          setAgentState('responding');
          idleTimeoutRef.current = setTimeout(() => setAgentState('idle'), 3000);
          // Trigger lip sync: generate visemes from response text
          // Skip when auto_spoken — the kernel already triggered VOICEVOX speech
          if (evtData.content && agentId && !evtData.auto_spoken) {
            api
              .generateVisemes(agentId, evtData.content as string, apiKey)
              .then((timeline) => {
                controllerRef.current?.playVisemes(timeline.entries);
              })
              .catch((err) => console.warn('[VRM] Viseme generation failed:', err));
          }
          break;
        case 'AgenticLoopCompleted':
          clearIdleTimeout();
          setAgentState('idle');
          // Don't kill active speech audio — only stop text-based visemes
          controllerRef.current?.stopVisemesSafe();
          break;
      }
    },
    apiKey,
  );

  function clearIdleTimeout() {
    if (idleTimeoutRef.current) {
      clearTimeout(idleTimeoutRef.current);
      idleTimeoutRef.current = null;
    }
  }

  useEffect(
    () => () => {
      clearIdleTimeout();
      if (footerTimeoutRef.current) clearTimeout(footerTimeoutRef.current);
    },
    [clearIdleTimeout],
  );

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

    loader
      .load(vrmUrl)
      .then((vrm) => {
        if (disposed) {
          loader.dispose();
          return;
        }
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

  // Enable window dragging in Tauri (decorations: false) — upper half only
  useEffect(() => {
    if (!isTauri) return;
    const handleDrag = async (e: MouseEvent) => {
      if ((e.target as HTMLElement).tagName === 'CANVAS' && e.clientY < window.innerHeight / 2) {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        await getCurrentWindow().startDragging();
      }
    };
    document.addEventListener('mousedown', handleDrag);
    return () => document.removeEventListener('mousedown', handleDrag);
  }, []);

  // Show footer on lower-half click, auto-hide after inactivity
  function handleCanvasClick(e: React.MouseEvent) {
    const rect = e.currentTarget.getBoundingClientRect();
    const relY = e.clientY - rect.top;
    if (relY > rect.height / 2) {
      setShowFooter(true);
      resetFooterTimeout();
    }
  }

  function resetFooterTimeout() {
    if (footerTimeoutRef.current) clearTimeout(footerTimeoutRef.current);
    footerTimeoutRef.current = setTimeout(() => {
      if (!showSettings) setShowFooter(false);
    }, 5000);
  }

  function handleFooterInteraction() {
    resetFooterTimeout();
  }

  function handlePoseClick(poseName: string) {
    controllerRef.current?.setPose(poseName, 0.5);
    setActivePose(poseName);
    // Sync sliders to the preset values
    const preset = POSE_PRESETS[poseName];
    if (preset) setPoseValues({ ...preset });
  }

  const handleSliderChange = useCallback((key: keyof DefaultPoseParams, value: number) => {
    setPoseValues((prev) => {
      const next = { ...prev, [key]: value };
      controllerRef.current?.setDirectPose(next);
      return next;
    });
    setActivePose(''); // deselect preset
  }, []);

  function toggleSettings() {
    setShowSettings((s) => !s);
    resetFooterTimeout();
  }

  function handleCopyValues() {
    // Format as TypeScript object for easy paste into types.ts
    const lines = Object.entries(poseValues)
      .map(([k, v]) => `  ${k}: ${typeof v === 'number' ? v.toFixed(2) : v},`)
      .join('\n');
    const text = `{\n${lines}\n}`;
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }

  async function handleVrmaFile(file: File) {
    if (!file.name.endsWith('.vrma') && !file.name.endsWith('.glb')) return;
    try {
      await controllerRef.current?.loadVrmaPoseFile(file, 0.5);
      setVrmaName(file.name);
      setActivePose(''); // Deselect preset
    } catch (err) {
      console.warn('[VRM] VRMA load failed:', err);
    }
  }

  function handleVrmaInput(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (file) handleVrmaFile(file);
    e.target.value = ''; // Allow re-selecting same file
  }

  function handleDrop(e: React.DragEvent) {
    e.preventDefault();
    setIsDragOver(false);
    const file = e.dataTransfer.files[0];
    if (file) handleVrmaFile(file);
  }

  function handleDragOver(e: React.DragEvent) {
    e.preventDefault();
    setIsDragOver(true);
  }

  function handleDragLeave() {
    setIsDragOver(false);
  }

  function handleStopVrma() {
    controllerRef.current?.stopVrma(0.5);
    setVrmaName(null);
    setActivePose('relaxed');
  }

  return (
    <div className="w-screen h-screen bg-transparent overflow-hidden relative flex flex-col">
      {/* Main content area */}
      <div className="flex-1 relative min-h-0">
        {/* Canvas area */}
        <div
          className="w-full h-full relative"
          onClick={handleCanvasClick}
          onDrop={handleDrop}
          onDragOver={handleDragOver}
          onDragLeave={handleDragLeave}
        >
          <canvas ref={canvasRef} className="w-full h-full" style={{ display: loading || error ? 'none' : 'block' }} />

          {/* VRMA drag & drop overlay */}
          {isDragOver && (
            <div className="absolute inset-0 flex items-center justify-center bg-brand/20 border-2 border-dashed border-brand rounded-lg z-20 pointer-events-none">
              <p className="text-white font-mono text-sm tracking-wider">Drop .vrma file</p>
            </div>
          )}

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
            <div className="absolute bottom-2 left-3 px-2.5 py-1 rounded-full text-[10px] font-mono uppercase tracking-widest bg-black/50 border border-white/10 text-white/80 backdrop-blur-sm">
              <span
                className={`inline-block w-1.5 h-1.5 rounded-full mr-1.5 ${
                  agentState === 'thinking' ? 'bg-blue-400 animate-pulse' : 'bg-brand animate-pulse'
                }`}
              />
              {agentState}
            </div>
          )}

          {/* Floating settings panel overlay */}
          {showSettings && showFooter && !loading && !error && (
            <div
              className="absolute top-0 left-0 bottom-0 w-56 bg-black/80 backdrop-blur-xl border-r border-white/10 overflow-y-auto z-10"
              onMouseMove={handleFooterInteraction}
              onClick={(e) => {
                e.stopPropagation();
                handleFooterInteraction();
              }}
            >
              <div className="p-2 space-y-3">
                <button
                  onClick={handleCopyValues}
                  className="w-full flex items-center justify-center gap-1.5 px-2 py-1 rounded text-[10px] font-mono bg-white/10 hover:bg-white/20 text-white/70 hover:text-white transition-colors"
                >
                  <Copy size={10} />
                  {copied ? 'Copied!' : 'Copy Values'}
                </button>

                {SLIDER_GROUPS.map(({ group, sliders }) => (
                  <div key={group}>
                    <div className="text-[9px] font-mono uppercase tracking-widest text-brand mb-1.5">{group}</div>
                    {sliders.map(({ key, label, min, max, step }) => (
                      <div key={key} className="mb-1">
                        <div className="flex items-center justify-between mb-0.5">
                          <span className="text-[9px] font-mono text-white/50">{label}</span>
                          <span className="text-[9px] font-mono text-white/80 tabular-nums w-10 text-right">
                            {poseValues[key].toFixed(2)}
                          </span>
                        </div>
                        <input
                          type="range"
                          min={min}
                          max={max}
                          step={step}
                          value={poseValues[key]}
                          onChange={(e) => handleSliderChange(key, parseFloat(e.target.value))}
                          className="w-full h-1 appearance-none bg-white/10 rounded-full outline-none
                            [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:w-2.5 [&::-webkit-slider-thumb]:h-2.5
                            [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-brand [&::-webkit-slider-thumb]:cursor-pointer
                            [&::-webkit-slider-thumb]:shadow-[0_0_4px_rgba(var(--brand-primary)/0.5)]"
                        />
                      </div>
                    ))}
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Bottom control bar + brand line — hidden by default, shown on lower-half click */}
      <div
        className={`flex-shrink-0 transition-all duration-300 ${
          showFooter && !loading && !error ? 'max-h-20 opacity-100' : 'max-h-0 opacity-0 overflow-hidden'
        }`}
        onMouseMove={handleFooterInteraction}
        onClick={handleFooterInteraction}
      >
        <div className="bg-black/40 backdrop-blur-md border-t border-white/5 px-3 py-1.5 flex items-center justify-center gap-1.5">
          <button
            onClick={() => toggleSettings()}
            className={`p-1 rounded transition-colors mr-2 ${
              showSettings ? 'text-brand' : 'text-white/40 hover:text-white/70'
            }`}
            title="Pose Editor"
          >
            <Settings size={14} />
          </button>
          {Object.keys(POSE_PRESETS).map((name) => (
            <button
              key={name}
              onClick={() => handlePoseClick(name)}
              className={`px-2.5 py-0.5 rounded-full text-[10px] font-mono tracking-wider transition-all duration-200 ${
                activePose === name
                  ? 'bg-brand text-white shadow-[0_0_8px_rgba(var(--brand-primary)/0.4)]'
                  : 'text-white/50 hover:text-white/80 hover:bg-white/10'
              }`}
            >
              {POSE_LABELS[name] ?? name}
            </button>
          ))}

          {/* VRMA separator + load button */}
          <div className="w-px h-4 bg-white/10 mx-1" />
          <button
            onClick={() => vrmaInputRef.current?.click()}
            className="p-1 rounded text-white/40 hover:text-white/70 transition-colors"
            title="Load VRMA pose"
          >
            <Upload size={14} />
          </button>
          <input ref={vrmaInputRef} type="file" accept=".vrma,.glb" className="hidden" onChange={handleVrmaInput} />
          {vrmaName && (
            <button
              onClick={handleStopVrma}
              className="px-2 py-0.5 rounded-full text-[10px] font-mono tracking-wider bg-brand/30 text-brand hover:bg-brand/50 transition-colors"
              title="Stop VRMA, return to preset pose"
            >
              {vrmaName} ✕
            </button>
          )}
        </div>
        <div className="h-[2px] bg-brand" />
      </div>
    </div>
  );
}
