import * as THREE from 'three';
import { VRM } from '@pixiv/three-vrm';
import { AvatarAgentState, IdleBehaviorParams, DefaultPoseParams, DEFAULT_IDLE_PARAMS, POSE_PRESETS } from './types';
import { VrmSceneManager } from './VrmSceneManager';
import { ProceduralBreathing } from './ProceduralBreathing';
import { ProceduralBlinking } from './ProceduralBlinking';
import { ProceduralMicroSway } from './ProceduralMicroSway';
import { ProceduralGazeDrift } from './ProceduralGazeDrift';
import { AgentStateAnimator } from './AgentStateAnimator';
import { DefaultPoseApplicator } from './DefaultPoseApplicator';
import { VisemePlayer, type VisemeEntry } from './VisemePlayer';
import { AudioPlaybackManager } from './AudioPlaybackManager';
import { VrmExpressionMapper } from './VrmExpressionMapper';
import { VrmaLoader } from './VrmaLoader';
import type { VRMAnimation } from '@pixiv/three-vrm-animation';

/**
 * Orchestrates all procedural animation layers.
 * Manages the requestAnimationFrame loop and coordinates VRM bone updates.
 */
/** VRMA-backed pose presets — loaded from public/ and cached after first use. */
const VRMA_POSE_URLS: Record<string, string> = {
  thinking: '/vrma/thinking.vrma',
};

export class VrmAnimationController {
  private sceneManager: VrmSceneManager;
  private vrm: VRM | null = null;
  private animFrameId: number | null = null;
  private clock = new THREE.Clock(false);
  private params: IdleBehaviorParams = { ...DEFAULT_IDLE_PARAMS };

  // Animation layers
  private defaultPose = new DefaultPoseApplicator();
  private breathing = new ProceduralBreathing();
  private blinking = new ProceduralBlinking();
  private microSway = new ProceduralMicroSway();
  private gazeDrift = new ProceduralGazeDrift();
  private stateAnimator = new AgentStateAnimator();
  private visemePlayer = new VisemePlayer();
  private audioManager = new AudioPlaybackManager();
  private expressionMapper = new VrmExpressionMapper();
  private vrmaLoader = new VrmaLoader();
  /** Pre-phoneme silence offset in ms (from VOICEVOX prePhonemeLength). */
  private audioOffsetMs = 0;
  /** Cache for VRMA-backed preset poses (loaded once, reused). */
  private vrmaPresetCache = new Map<string, VRMAnimation>();
  /** True when current VRMA is from a preset (not user-loaded). */
  private isPresetVrma = false;

  private _running = false;

  constructor(sceneManager: VrmSceneManager) {
    this.sceneManager = sceneManager;

    // Pause on tab hidden
    document.addEventListener('visibilitychange', this.handleVisibility);
  }

  setVrm(vrm: VRM) {
    this.vrm = vrm;
    this.expressionMapper.initialize(vrm);
    this.visemePlayer.setMapper(this.expressionMapper);
    this.blinking.setMapper(this.expressionMapper);
    this.vrmaLoader.setVrm(vrm);
  }

  setAgentState(state: AvatarAgentState) {
    this.stateAnimator.setState(state);
  }

  setIdleParams(params: IdleBehaviorParams) {
    this.params = { ...params };
    this.defaultPose.setParams(params.pose);
  }

  playVisemes(timeline: VisemeEntry[]) {
    this.visemePlayer.play(timeline);
  }

  /** Play audio from URL with synchronized lip sync visemes. */
  async playSpeech(audioUrl: string, visemeTimeline: VisemeEntry[], audioOffsetMs = 0) {
    try {
      this.audioOffsetMs = audioOffsetMs;
      // Decode and start audio FIRST, then start visemes in sync.
      // This prevents visemes from running ahead during decode latency.
      await this.audioManager.play(audioUrl);
      this.visemePlayer.playSync(visemeTimeline);
    } catch (err) {
      console.warn('[VRM] Speech playback failed:', err);
      this.visemePlayer.stop();
    }
  }

  /** Play inline base64 audio with synchronized lip sync visemes. */
  async playSpeechData(base64Data: string, visemeTimeline: VisemeEntry[], audioOffsetMs = 0) {
    try {
      this.audioOffsetMs = audioOffsetMs;
      await this.audioManager.playData(base64Data);
      this.visemePlayer.playSync(visemeTimeline);
    } catch (err) {
      console.warn('[VRM] Speech playback failed:', err);
      this.visemePlayer.stop();
    }
  }

  /** Returns true if speech audio is actively playing. */
  isSpeechPlaying(): boolean {
    return this.audioManager.isPlaying();
  }

  stopVisemes() {
    this.visemePlayer.stop();
    this.audioManager.stop();
  }

  /** Stop only non-speech visemes; leaves active speech audio untouched. */
  stopVisemesSafe() {
    if (!this.audioManager.isPlaying()) {
      this.visemePlayer.stop();
    }
  }

  /** Set VRM expression (from MGP avatar server). */
  setExpression(name: string, intensity: number) {
    if (!this.vrm?.expressionManager) return;
    const resolved = this.expressionMapper.resolveExpression(name) ?? name;
    this.vrm.expressionManager.setValue(resolved, intensity);
  }

  /** Set pose params directly without transition (for real-time sliders). */
  setDirectPose(params: DefaultPoseParams) {
    this.defaultPose.setParams(params);
  }

  /** Map pose names to agent states for synchronized behavior (eye close, etc.). */
  private static readonly POSE_STATE_MAP: Partial<Record<string, AvatarAgentState>> = {
    thinking: 'thinking',
  };

  /** Transition to a named preset pose (from MGP avatar server). */
  async setPose(name: string, transitionSec = 0.5) {
    // Sync agent state with pose (e.g. thinking → eyes closed)
    this.stateAnimator.setState(VrmAnimationController.POSE_STATE_MAP[name] ?? 'idle');

    // Check for VRMA-backed preset (higher priority than DefaultPoseParams)
    const vrmaUrl = VRMA_POSE_URLS[name];
    if (vrmaUrl) {
      let animation = this.vrmaPresetCache.get(name);
      if (!animation) {
        try {
          animation = await this.vrmaLoader.load(vrmaUrl);
          this.vrmaPresetCache.set(name, animation);
        } catch (err) {
          console.warn(`[VRM] Failed to load VRMA preset "${name}", falling back:`, err);
          // Fall through to DefaultPoseApplicator below
        }
      }
      if (animation) {
        this.vrmaLoader.applyPose(animation, transitionSec);
        this.isPresetVrma = true;
        return;
      }
    }

    // DefaultPoseApplicator-based pose — fade out VRMA smoothly
    if (this.vrmaLoader.active) {
      this.vrmaLoader.stop(transitionSec);
      this.isPresetVrma = false;
    }

    const preset = POSE_PRESETS[name];
    if (!preset) {
      console.warn(`[VRM] Unknown pose preset: ${name}`);
      return;
    }
    this.defaultPose.transitionTo(preset, transitionSec);
  }

  /** Load a VRMA file from a File object and apply as static pose. */
  async loadVrmaPoseFile(file: File, transitionSec = 0.5): Promise<VRMAnimation> {
    const animation = await this.vrmaLoader.loadFile(file);
    this.vrmaLoader.applyPose(animation, transitionSec);
    this.isPresetVrma = false;
    return animation;
  }

  /** Load a VRMA file from URL and apply as static pose. */
  async loadVrmaPose(url: string, transitionSec = 0.5): Promise<VRMAnimation> {
    const animation = await this.vrmaLoader.load(url);
    this.vrmaLoader.applyPose(animation, transitionSec);
    this.isPresetVrma = false;
    return animation;
  }

  /** Load a VRMA file from a File object and play as animation. */
  async loadVrmaAnimationFile(file: File, transitionSec = 0.5): Promise<VRMAnimation> {
    const animation = await this.vrmaLoader.loadFile(file);
    this.vrmaLoader.playAnimation(animation, transitionSec);
    this.isPresetVrma = false;
    return animation;
  }

  /** Stop VRMA playback and return to DefaultPoseApplicator. */
  stopVrma(transitionSec = 0.5) {
    this.vrmaLoader.stop(transitionSec);
    this.isPresetVrma = false;
  }

  /** True when a user-loaded VRMA pose/animation is controlling the base pose. */
  get isVrmaActive(): boolean {
    return this.vrmaLoader.active && !this.isPresetVrma;
  }

  /** Update idle behavior parameters (from MGP avatar server). */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  setIdleBehavior(params: Record<string, any>) {
    if (params.mode) this.params.mode = params.mode;
    if (params.breathing_rate !== undefined) this.params.breathing_rate = params.breathing_rate;
    if (params.sway_amplitude !== undefined) this.params.sway_amplitude = params.sway_amplitude;
    if (params.blink_frequency !== undefined) this.params.blink_frequency = params.blink_frequency;
    if (params.pose) this.defaultPose.setParams(params.pose);
  }

  start() {
    if (this._running) return;
    this._running = true;
    this.clock.start();
    this.tick();
  }

  stop() {
    this._running = false;
    this.clock.stop();
    if (this.animFrameId !== null) {
      cancelAnimationFrame(this.animFrameId);
      this.animFrameId = null;
    }
  }

  private tick = () => {
    if (!this._running) return;
    this.animFrameId = requestAnimationFrame(this.tick);

    const deltaTime = Math.min(this.clock.getDelta(), 0.1); // Cap at 100ms
    if (deltaTime <= 0 || !this.vrm) return;

    // 1. Reset bones to rest pose (so layers are additive from neutral)
    this.vrm.humanoid?.resetNormalizedPose();

    // 2. Apply base pose
    //    DefaultPose always runs as the base layer.
    //    VRMA pose mode slerps on top (influence 0→1 for smooth transition).
    //    VRMA animation mode (mixer) overwrites directly.
    this.defaultPose.update(deltaTime);
    this.defaultPose.apply(this.vrm);

    if (this.vrmaLoader.active) {
      this.vrmaLoader.update(deltaTime);
    }

    // 3. Apply procedural layers
    const swayDamping = this.stateAnimator.swayDamping;
    this.breathing.update(this.vrm, deltaTime, this.params.breathing_rate);
    this.blinking.update(this.vrm, deltaTime, this.params.blink_frequency);
    this.microSway.update(this.vrm, deltaTime, this.params.sway_amplitude * swayDamping);

    // 4. Apply agent state modifiers (head tilt, spine lean)
    this.stateAnimator.update(this.vrm, deltaTime);

    // 5. Gaze — compose mouse target with agent state Y offset
    const gazeTarget = this.sceneManager.mouseTarget.clone();
    gazeTarget.y += this.stateAnimator.gazeYOffset;
    this.gazeDrift.update(this.vrm, deltaTime, gazeTarget);

    // 5.5. Apply lip sync visemes (sync to audio clock when playing speech)
    //       Subtract audioOffsetMs to skip pre-phoneme silence in the WAV.
    if (this.audioManager.isPlaying()) {
      this.visemePlayer.setExternalTime(
        Math.max(0, this.audioManager.getCurrentTimeMs() - this.audioOffsetMs),
      );
    } else if (this.visemePlayer.isPlaying() && this.visemePlayer.isSynced()) {
      // Audio ended naturally while visemes were in sync mode → stop visemes
      this.visemePlayer.stop();
    }
    this.visemePlayer.update(this.vrm, deltaTime);

    // 6. Update VRM (SpringBone physics, expression apply, normalized → raw copy)
    this.vrm.update(deltaTime);

    // 7. Render
    this.sceneManager.render();
  };

  private handleVisibility = () => {
    if (document.hidden) {
      this.stop();
    } else if (this.vrm) {
      this.start();
    }
  };

  dispose() {
    this.stop();
    this.audioManager.dispose();
    this.vrmaLoader.dispose();
    document.removeEventListener('visibilitychange', this.handleVisibility);
    if (this.vrm) {
      this.breathing.reset(this.vrm);
    }
    this.vrm = null;
  }
}
