import type { VRM } from '@pixiv/three-vrm';
import type { VRMAnimation } from '@pixiv/three-vrm-animation';
import * as THREE from 'three';
import { AgentStateAnimator } from './AgentStateAnimator';
import { AudioPlaybackManager } from './AudioPlaybackManager';
import { CompanionMotion } from './CompanionMotion';
import {
  type CompanionGesture,
  DEFAULT_COMPANION_IDLE,
  type MotionStyle,
  smoothMotion,
} from './companionMotionLibrary';
import { DefaultPoseApplicator } from './DefaultPoseApplicator';
import { importMotion } from './MotionImport';
import { ProceduralBlinking } from './ProceduralBlinking';
import { ProceduralBreathing } from './ProceduralBreathing';
import { ProceduralGazeDrift } from './ProceduralGazeDrift';
import { ProceduralMicroSway } from './ProceduralMicroSway';
import {
  type AvatarAgentState,
  DEFAULT_IDLE_PARAMS,
  type DefaultPoseParams,
  type IdleBehaviorParams,
  POSE_PRESETS,
} from './types';
import { type VisemeEntry, VisemePlayer } from './VisemePlayer';
import { VrmaLoader } from './VrmaLoader';
import { VrmExpressionMapper } from './VrmExpressionMapper';
import type { VrmSceneManager } from './VrmSceneManager';

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
  private params: IdleBehaviorParams = { ...DEFAULT_COMPANION_IDLE };
  private motionStyle: MotionStyle = 'companion';
  private companion = new CompanionMotion();
  private reducedMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)');
  private blendFrom = new Map<THREE.Object3D, THREE.Quaternion>();
  private blendElapsed = 1;
  private blendDuration = 0.7;
  private blendPositions = new Map<THREE.Object3D, THREE.Vector3>();
  private originalChestScale: THREE.Vector3 | null = null;
  private poseRequest = 0;
  private comparisonFilename: string | null = null;

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
  /** Tracks current agent state for transition detection. */
  private currentAgentState: AvatarAgentState = 'idle';

  constructor(sceneManager: VrmSceneManager) {
    this.sceneManager = sceneManager;

    // Pause on tab hidden
    document.addEventListener('visibilitychange', this.handleVisibility);
  }

  setVrm(vrm: VRM) {
    this.comparisonFilename = null;
    this.vrm = vrm;
    this.expressionMapper.initialize(vrm);
    this.visemePlayer.setMapper(this.expressionMapper);
    this.blinking.setMapper(this.expressionMapper);
    this.vrmaLoader.setVrm(vrm);
    this.companion.setVrm(vrm, this.expressionMapper);
    this.originalChestScale = vrm.humanoid?.getRawBoneNode('chest')?.scale.clone() ?? null;
  }

  setMotionStyle(style: MotionStyle) {
    if (style === this.motionStyle) return;
    this.poseRequest++;
    this.captureBlend(0.7);
    if (this.isPresetVrma) this.stopVrma(0);
    if (this.originalChestScale) this.vrm?.humanoid?.getRawBoneNode('chest')?.scale.copy(this.originalChestScale);
    this.motionStyle = style;
    this.params = { ...(style === 'legacy' ? DEFAULT_IDLE_PARAMS : DEFAULT_COMPANION_IDLE) };
    this.defaultPose.setParams(this.params.pose);
    this.companion.setPoseParams(this.params.pose);
  }

  private captureBlend(duration: number) {
    this.blendFrom.clear();
    this.blendPositions.clear();
    for (const bone of Object.values(this.vrm?.humanoid?.normalizedHumanBones ?? {}))
      if (bone) {
        this.blendFrom.set(bone.node, bone.node.quaternion.clone());
        this.blendPositions.set(bone.node, bone.node.position.clone());
      }
    this.blendElapsed = 0;
    this.blendDuration = Math.max(duration, 0.7);
  }

  previewGesture(name: CompanionGesture) {
    if (this.motionStyle === 'companion' && !this.isVrmaActive) this.companion.playGesture(name);
  }

  setAgentState(state: AvatarAgentState) {
    const prev = this.currentAgentState;
    this.currentAgentState = state;
    this.stateAnimator.setState(state);
    this.companion.setState(state);
    if (this.motionStyle === 'companion') return;

    // Don't override user-loaded VRMA (manual pose/animation)
    if (this.vrmaLoader.active && !this.isPresetVrma) return;

    if (state === prev) return;

    // Apply VRMA preset for the new state, or stop it when leaving
    if (state in VRMA_POSE_URLS) {
      this.setPose(state, 0.5);
    } else if (prev in VRMA_POSE_URLS && this.isPresetVrma) {
      this.stopVrma(0.5);
    }
  }

  setIdleParams(params: IdleBehaviorParams) {
    const poseChanged = Object.keys(params.pose).some(
      (key) => params.pose[key as keyof DefaultPoseParams] !== this.params.pose[key as keyof DefaultPoseParams],
    );
    this.params = { ...params };
    this.defaultPose.setParams(params.pose);
    if (poseChanged) this.companion.setPoseParams(params.pose);
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
      if (import.meta.env.DEV) console.warn('[VRM] Speech playback failed:', err);
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
      if (import.meta.env.DEV) console.warn('[VRM] Speech playback failed:', err);
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
    this.companion.setPoseParams(params, 0.12);
  }

  /** Map pose names to agent states for synchronized behavior (eye close, etc.). */
  private static readonly POSE_STATE_MAP: Partial<Record<string, AvatarAgentState>> = {
    thinking: 'thinking',
  };

  /** Transition to a named preset pose (from MGP avatar server). */
  async setPose(name: string, transitionSec = 0.5) {
    const request = ++this.poseRequest;
    if (this.motionStyle === 'companion') {
      if (this.vrmaLoader.active) this.stopVrma(transitionSec);
      else if (this.blendElapsed < this.blendDuration) this.captureBlend(Math.max(transitionSec, 0.75));
      this.companion.setPose(name, Math.max(transitionSec, 0.75));
      return;
    }
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
          if (import.meta.env.DEV) console.warn(`[VRM] Failed to load VRMA preset "${name}", falling back:`, err);
          // Fall through to DefaultPoseApplicator below
        }
      }
      if (animation) {
        if (request !== this.poseRequest || this.motionStyle !== 'legacy') return;
        this.vrmaLoader.applyPose(animation, transitionSec);
        this.isPresetVrma = true;
        return;
      }
    }

    // DefaultPoseApplicator-based pose — fade out VRMA smoothly
    if (request !== this.poseRequest) return;
    if (this.vrmaLoader.active) {
      this.vrmaLoader.stop(transitionSec);
      this.isPresetVrma = false;
    }

    const preset = POSE_PRESETS[name];
    if (!preset) {
      if (import.meta.env.DEV) console.warn(`[VRM] Unknown pose preset: ${name}`);
      return;
    }
    this.defaultPose.transitionTo(preset, transitionSec);
  }

  /** Load a VRMA file from a File object and apply as static pose. */
  async loadVrmaPoseFile(file: File, transitionSec = 0.5): Promise<VRMAnimation> {
    const request = ++this.poseRequest;
    const animation = await this.vrmaLoader.loadFile(file);
    if (request !== this.poseRequest) return animation;
    this.releaseProceduralFace();
    this.vrmaLoader.applyPose(animation, transitionSec);
    this.comparisonFilename = null;
    this.isPresetVrma = false;
    return animation;
  }

  /** Load a VRMA file from URL and apply as static pose. */
  async loadVrmaPose(url: string, transitionSec = 0.5): Promise<VRMAnimation> {
    const request = ++this.poseRequest;
    const animation = await this.vrmaLoader.load(url);
    if (request !== this.poseRequest) return animation;
    this.releaseProceduralFace();
    this.vrmaLoader.applyPose(animation, transitionSec);
    this.comparisonFilename = null;
    this.isPresetVrma = false;
    return animation;
  }

  /** Load a VRMA file from a File object and play as animation. */
  async loadVrmaAnimationFile(file: File, transitionSec = 0.5): Promise<VRMAnimation> {
    const request = ++this.poseRequest;
    const animation = await this.vrmaLoader.loadFile(file);
    if (request !== this.poseRequest) return animation;
    this.releaseProceduralFace();
    this.vrmaLoader.playAnimation(animation, transitionSec);
    this.comparisonFilename = null;
    this.isPresetVrma = false;
    return animation;
  }

  /** Parse before applying, so unsupported files leave the displayed motion intact. */
  async loadComparisonMotion(file: File): Promise<VRMAnimation> {
    const request = ++this.poseRequest;
    const animation = await importMotion(file, (source) => this.vrmaLoader.loadFile(source));
    if (request !== this.poseRequest) throw new Error('Motion selection changed');
    this.releaseProceduralFace();
    this.vrmaLoader.playAnimation(animation, 0);
    this.comparisonFilename = file.name;
    this.isPresetVrma = false;
    return animation;
  }

  get motionPlayback() {
    return this.vrmaLoader.playback;
  }
  get comparisonMotionName() {
    return this.vrmaLoader.active ? this.comparisonFilename : null;
  }
  pauseMotion(paused: boolean) {
    this.vrmaLoader.setPaused(paused);
  }
  setMotionSpeed(speed: number) {
    this.vrmaLoader.setSpeed(speed);
  }
  seekMotion(seconds: number) {
    this.vrmaLoader.seek(seconds);
  }

  private releaseProceduralFace() {
    if (this.motionStyle !== 'companion') return;
    for (const name of this.expressionMapper.getBlinkNames()) this.vrm?.expressionManager?.setValue(name, 0);
    if (this.vrm?.lookAt) this.vrm.lookAt.target = null;
  }

  /** Stop VRMA playback and return to DefaultPoseApplicator. */
  stopVrma(transitionSec = 0.5) {
    this.poseRequest++;
    this.comparisonFilename = null;
    if (this.motionStyle === 'companion' && this.vrmaLoader.active) {
      // Capture the displayed pose before AnimationMixer restores its original bindings.
      this.captureBlend(transitionSec);
      this.vrmaLoader.stop(0);
      for (const [node, q] of this.blendFrom) {
        node.quaternion.copy(q);
        const position = this.blendPositions.get(node);
        if (position) node.position.copy(position);
      }
    } else this.vrmaLoader.stop(transitionSec);
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
    if (params.pose) this.setDirectPose(params.pose);
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

    this.updateFrame(deltaTime);
  };

  /** Advance a frame independently of the display refresh rate. */
  updateFrame(deltaTime: number) {
    if (!this.vrm || deltaTime <= 0) return;
    // AnimationMixer caches constant tracks. Resetting those joints between frames erases its authored pose.
    const comparison = Boolean(this.comparisonMotionName);
    const importedAnimation =
      (this.motionStyle === 'companion' || comparison) && this.vrmaLoader.active && !this.vrmaLoader.isPose;
    if (!importedAnimation) this.vrm.humanoid?.resetNormalizedPose();

    // 2. Apply base pose
    //    DefaultPose always runs as the base layer.
    //    VRMA pose mode slerps on top (influence 0→1 for smooth transition).
    //    VRMA animation mode (mixer) overwrites directly.
    if (comparison) {
      // Evaluate the authored tracks alone, including when original movement is selected.
    } else if (this.motionStyle === 'companion') {
      this.companion.advance(deltaTime);
      if (!importedAnimation) this.companion.applyBase();
    } else {
      this.defaultPose.update(deltaTime);
      this.defaultPose.apply(this.vrm);
    }

    if (this.vrmaLoader.active) {
      this.vrmaLoader.update(deltaTime);
    }

    if (comparison) {
      // Procedural joint motion would contaminate paused frames and comparisons.
    } else if (this.motionStyle === 'companion') {
      this.companion.applyLife(
        deltaTime,
        this.params,
        this.sceneManager.mouseTarget,
        this.reducedMotion?.matches ?? false,
        this.vrmaLoader.active ? (this.vrmaLoader.isPose ? 'pose' : true) : false,
      );
    } else {
      // 3. Apply original procedural layers
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
    }

    if (this.blendElapsed < this.blendDuration && !this.isVrmaActive) {
      this.blendElapsed += deltaTime;
      const weight = smoothMotion(this.blendElapsed / this.blendDuration);
      for (const [node, from] of this.blendFrom) {
        node.quaternion.slerpQuaternions(from, node.quaternion.clone(), weight);
        const position = this.blendPositions.get(node);
        if (position) node.position.lerpVectors(position, node.position.clone(), weight);
      }
    }
    if (this.motionStyle === 'companion') this.companion.rememberRenderedPose();

    // 5.5. Apply lip sync visemes (sync to audio clock when playing speech)
    //       Subtract audioOffsetMs to skip pre-phoneme silence in the WAV.
    if (this.audioManager.isPlaying()) {
      this.visemePlayer.setExternalTime(Math.max(0, this.audioManager.getCurrentTimeMs() - this.audioOffsetMs));
    } else if (this.visemePlayer.isPlaying() && this.visemePlayer.isSynced()) {
      // Audio ended naturally while visemes were in sync mode → stop visemes
      this.visemePlayer.stop();
    }
    this.visemePlayer.update(this.vrm, deltaTime);

    // 6. Update VRM (SpringBone physics, expression apply, normalized → raw copy)
    this.vrm.update(deltaTime);

    if (this.motionStyle === 'companion') {
      let highest = -Infinity;
      for (const side of ['left', 'right'] as const)
        for (const name of [`${side}Hand`, `${side}MiddleDistal`, `${side}IndexDistal`] as const) {
          const bone = this.vrm.humanoid?.getNormalizedBoneNode(name);
          if (bone) highest = Math.max(highest, bone.getWorldPosition(new THREE.Vector3()).y + 0.025);
        }
      this.sceneManager.updateMotionFraming?.(highest, deltaTime);
    } else this.sceneManager.updateMotionFraming?.(-1, deltaTime);

    // 7. Render
    this.sceneManager.render();
  }

  private handleVisibility = () => {
    if (document.hidden) {
      this.stop();
    } else if (this.vrm) {
      this.start();
    }
  };

  dispose() {
    this.poseRequest++;
    this.stop();
    this.audioManager.dispose();
    this.vrmaLoader.dispose();
    document.removeEventListener('visibilitychange', this.handleVisibility);
    if (this.vrm) {
      this.breathing.reset(this.vrm);
      if (this.originalChestScale) this.vrm.humanoid?.getRawBoneNode('chest')?.scale.copy(this.originalChestScale);
    }
    this.vrm = null;
  }
}
