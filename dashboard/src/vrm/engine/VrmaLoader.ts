import * as THREE from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { VRM } from '@pixiv/three-vrm';
import {
  VRMAnimation,
  VRMAnimationLoaderPlugin,
  createVRMAnimationClip,
} from '@pixiv/three-vrm-animation';

/**
 * Loads VRMA files and applies them to VRM.
 *
 * For static poses: directly applies quaternions via slerp blending each frame.
 *   DefaultPoseApplicator must run BEFORE update() so slerp blends from DefaultPose → VRMA.
 * For animations: uses AnimationMixer for proper playback with looping.
 *
 * When active, the caller must call update() each frame after the base pose is applied.
 * vrm.update() must still be called afterwards to copy normalized → raw.
 */
export class VrmaLoader {
  private loader: GLTFLoader;
  private mixer: THREE.AnimationMixer | null = null;
  private currentAction: THREE.AnimationAction | null = null;
  private vrm: VRM | null = null;

  /** Direct pose targets — bypasses AnimationMixer for reliable static pose application. */
  private poseTargets: { node: THREE.Object3D; quaternion: THREE.Quaternion }[] = [];
  private _isPoseMode = false;

  /** Influence blending for smooth transitions (0 = DefaultPose, 1 = VRMA). */
  private influence = 0;
  private transitionFrom = 0;
  private transitionTo = 1;
  private transitionDuration = 0.5;
  private transitionElapsed = 0;

  /** True when a VRMA pose/animation is actively controlling bones. */
  private _active = false;
  private _stopping = false;

  constructor() {
    this.loader = new GLTFLoader();
    this.loader.register((parser) => new VRMAnimationLoaderPlugin(parser));
  }

  get active(): boolean {
    return this._active;
  }

  get isPose(): boolean {
    return this._isPoseMode;
  }

  setVrm(vrm: VRM) {
    this.vrm = vrm;
    this.mixer = new THREE.AnimationMixer(vrm.scene);
  }

  /** Load a VRMA from URL and return the parsed VRMAnimation. */
  async load(url: string): Promise<VRMAnimation> {
    const gltf = await this.loader.loadAsync(url);
    const animations: VRMAnimation[] | undefined = gltf.userData.vrmAnimations;
    if (!animations || animations.length === 0) {
      throw new Error('No VRM animation found in file');
    }
    return animations[0];
  }

  /** Load a VRMA from a local File (e.g. drag & drop). */
  async loadFile(file: File): Promise<VRMAnimation> {
    const url = URL.createObjectURL(file);
    try {
      return await this.load(url);
    } finally {
      URL.revokeObjectURL(url);
    }
  }

  /**
   * Apply a VRMAnimation as a static pose (frozen at frame 0).
   * Smoothly transitions from the current base pose via slerp blending.
   */
  applyPose(animation: VRMAnimation, transitionSec = 0.5) {
    if (!this.vrm) return;

    const clip = createVRMAnimationClip(animation, this.vrm);

    // Extract quaternion values at frame 0 and resolve target nodes directly
    this.poseTargets = [];
    for (const track of clip.tracks) {
      if (track.name.endsWith('.quaternion') && track.values.length >= 4) {
        const objectName = track.name.replace('.quaternion', '');
        const node = this.vrm.scene.getObjectByName(objectName);
        if (node) {
          this.poseTargets.push({
            node,
            quaternion: new THREE.Quaternion(
              track.values[0], track.values[1], track.values[2], track.values[3],
            ),
          });
        }
      }
    }

    // Stop any running mixer animation
    this.mixer?.stopAllAction();
    this.currentAction = null;

    this._isPoseMode = true;
    this._active = true;
    this._stopping = false;

    // Start influence transition: 0 → 1 (DefaultPose → VRMA)
    this.transitionFrom = this.influence; // smooth chain if already transitioning
    this.transitionTo = 1;
    this.transitionDuration = Math.max(transitionSec, 0.01);
    this.transitionElapsed = 0;
  }

  /** Play a VRMAnimation as a looping animation. */
  playAnimation(animation: VRMAnimation, transitionSec = 0.5) {
    if (!this.vrm || !this.mixer) return;

    // Clear any pose mode
    this.poseTargets = [];
    this._isPoseMode = false;
    this.influence = 0;

    const clip = createVRMAnimationClip(animation, this.vrm);
    const newAction = this.mixer.clipAction(clip);
    newAction.setLoop(THREE.LoopRepeat, Infinity);

    if (this.currentAction && transitionSec > 0) {
      newAction.crossFadeFrom(this.currentAction, transitionSec, true);
    } else {
      this.mixer.stopAllAction();
    }

    newAction.play();
    this.currentAction = newAction;
    this._active = true;
    this._stopping = false;
  }

  /** Stop VRMA playback and return control to DefaultPoseApplicator. */
  stop(transitionSec = 0.5) {
    if (this._isPoseMode) {
      if (transitionSec > 0 && this.poseTargets.length > 0) {
        // Fade out: transition influence from current → 0
        this._stopping = true;
        this.transitionFrom = this.influence;
        this.transitionTo = 0;
        this.transitionDuration = Math.max(transitionSec, 0.01);
        this.transitionElapsed = 0;
      } else {
        // Instant stop
        this.poseTargets = [];
        this._isPoseMode = false;
        this._active = false;
        this._stopping = false;
        this.influence = 0;
      }
      return;
    }

    if (!this.currentAction || !this.mixer) {
      this._active = false;
      return;
    }

    if (transitionSec > 0) {
      this.currentAction.fadeOut(transitionSec);
      this._stopping = true;
      setTimeout(() => {
        this.mixer?.stopAllAction();
        this.currentAction = null;
        this._active = false;
        this._stopping = false;
      }, transitionSec * 1000);
    } else {
      this.mixer.stopAllAction();
      this.currentAction = null;
      this._active = false;
    }
  }

  /**
   * Update each frame. For pose mode, call AFTER DefaultPoseApplicator has applied.
   * Slerps bone quaternions from DefaultPose toward VRMA targets based on influence.
   */
  update(deltaTime: number) {
    if (!this._active) return;

    // Advance influence transition (smoothstep easing)
    if (this.transitionElapsed < this.transitionDuration) {
      this.transitionElapsed = Math.min(
        this.transitionElapsed + deltaTime,
        this.transitionDuration,
      );
      const t = this.transitionElapsed / this.transitionDuration;
      const ease = t * t * (3 - 2 * t); // smoothstep
      this.influence = this.transitionFrom + (this.transitionTo - this.transitionFrom) * ease;

      // Fade-out completed → deactivate
      if (this._stopping && this.transitionElapsed >= this.transitionDuration) {
        this.poseTargets = [];
        this._isPoseMode = false;
        this._active = false;
        this._stopping = false;
        this.influence = 0;
        return;
      }
    }

    if (this._isPoseMode) {
      // Slerp from current bone quaternion (DefaultPose) toward VRMA target
      for (const { node, quaternion } of this.poseTargets) {
        node.quaternion.slerp(quaternion, this.influence);
      }
    } else if (this.mixer) {
      this.mixer.update(deltaTime);
    }
  }

  dispose() {
    this.mixer?.stopAllAction();
    this.mixer = null;
    this.currentAction = null;
    this.poseTargets = [];
    this._isPoseMode = false;
    this._active = false;
    this._stopping = false;
    this.influence = 0;
    this.vrm = null;
  }
}
