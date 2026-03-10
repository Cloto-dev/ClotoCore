import * as THREE from 'three';
import { VRM } from '@pixiv/three-vrm';
import { AvatarAgentState, IdleBehaviorParams, DEFAULT_IDLE_PARAMS } from './types';
import { VrmSceneManager } from './VrmSceneManager';
import { ProceduralBreathing } from './ProceduralBreathing';
import { ProceduralBlinking } from './ProceduralBlinking';
import { ProceduralMicroSway } from './ProceduralMicroSway';
import { ProceduralGazeDrift } from './ProceduralGazeDrift';
import { AgentStateAnimator } from './AgentStateAnimator';
import { DefaultPoseApplicator } from './DefaultPoseApplicator';
import { VisemePlayer, type VisemeEntry } from './VisemePlayer';
import { AudioPlaybackManager } from './AudioPlaybackManager';

/**
 * Orchestrates all procedural animation layers.
 * Manages the requestAnimationFrame loop and coordinates VRM bone updates.
 */
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

  private _running = false;

  constructor(sceneManager: VrmSceneManager) {
    this.sceneManager = sceneManager;

    // Pause on tab hidden
    document.addEventListener('visibilitychange', this.handleVisibility);
  }

  setVrm(vrm: VRM) {
    this.vrm = vrm;
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
  async playSpeech(audioUrl: string, visemeTimeline: VisemeEntry[]) {
    try {
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
  async playSpeechData(base64Data: string, visemeTimeline: VisemeEntry[]) {
    try {
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
    this.vrm.expressionManager.setValue(name, intensity);
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

    // 2. Apply default resting pose (base for all procedural layers)
    this.defaultPose.apply(this.vrm);

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
    if (this.audioManager.isPlaying()) {
      this.visemePlayer.setExternalTime(this.audioManager.getCurrentTimeMs());
    } else if (this.visemePlayer.isPlaying() && this.visemePlayer.isSynced()) {
      // Audio ended naturally while visemes were in sync mode → stop visemes
      this.visemePlayer.stop();
    }
    this.visemePlayer.update(this.vrm, deltaTime);

    // 6. Update VRM (SpringBone physics, expression apply)
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
    document.removeEventListener('visibilitychange', this.handleVisibility);
    if (this.vrm) {
      this.breathing.reset(this.vrm);
    }
    this.vrm = null;
  }
}
