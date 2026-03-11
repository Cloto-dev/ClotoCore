import * as THREE from 'three';
import { VRM } from '@pixiv/three-vrm';
import { AvatarAgentState } from './types';

/**
 * Applies agent-state-dependent bone modifiers on top of idle animation.
 * Smoothly transitions between states using lerp.
 */

interface StateModifiers {
  gazeYOffset: number;      // Added to gaze target Y
  headTiltZ: number;        // Head Z rotation (radians)
  spineLeanX: number;       // Spine X rotation (radians)
  swayDamping: number;      // Multiplier on sway amplitude (0-1)
  eyeClose: number;         // VRM blink expression (0 = open, 1 = closed)
}

const STATE_TARGETS: Record<AvatarAgentState, StateModifiers> = {
  idle: {
    gazeYOffset: 0,
    headTiltZ: 0,
    spineLeanX: 0,
    swayDamping: 1.0,
    eyeClose: 0,
  },
  thinking: {
    gazeYOffset: 0.15,        // Look slightly upward
    headTiltZ: 0.05,          // ~3 degree head tilt
    spineLeanX: 0,
    swayDamping: 0.5,         // Reduce sway while thinking
    eyeClose: 0.2,            // Eyes slightly narrowed
  },
  responding: {
    gazeYOffset: 0,
    headTiltZ: 0,
    spineLeanX: 0.035,        // ~2 degree forward lean
    swayDamping: 0.8,
    eyeClose: 0,
  },
};

const TRANSITION_SPEED = 3.0; // ~0.3s to reach target (1 - e^(-3*0.33) ≈ 0.63)

export class AgentStateAnimator {
  private currentState: AvatarAgentState = 'idle';
  private current: StateModifiers = { ...STATE_TARGETS.idle };

  /** Current gaze Y offset for external composition with mouse target */
  get gazeYOffset(): number {
    return this.current.gazeYOffset;
  }

  /** Current sway damping factor for external ProceduralMicroSway scaling */
  get swayDamping(): number {
    return this.current.swayDamping;
  }

  setState(state: AvatarAgentState) {
    this.currentState = state;
  }

  update(vrm: VRM, deltaTime: number) {
    const target = STATE_TARGETS[this.currentState];
    const lerpFactor = 1 - Math.exp(-TRANSITION_SPEED * deltaTime);

    // Interpolate all modifiers
    this.current.gazeYOffset = THREE.MathUtils.lerp(this.current.gazeYOffset, target.gazeYOffset, lerpFactor);
    this.current.headTiltZ = THREE.MathUtils.lerp(this.current.headTiltZ, target.headTiltZ, lerpFactor);
    this.current.spineLeanX = THREE.MathUtils.lerp(this.current.spineLeanX, target.spineLeanX, lerpFactor);
    this.current.swayDamping = THREE.MathUtils.lerp(this.current.swayDamping, target.swayDamping, lerpFactor);
    this.current.eyeClose = THREE.MathUtils.lerp(this.current.eyeClose, target.eyeClose, lerpFactor);

    // Apply bone modifiers
    const humanoid = vrm.humanoid;
    if (!humanoid) return;

    const head = humanoid.getNormalizedBoneNode('head');
    if (head) {
      head.rotation.z += this.current.headTiltZ;
    }

    const spine = humanoid.getNormalizedBoneNode('spine');
    if (spine) {
      spine.rotation.x += this.current.spineLeanX;
    }

    // Apply eye close via VRM blink expression
    if (this.current.eyeClose > 0.01) {
      vrm.expressionManager?.setValue('blink', this.current.eyeClose);
    }
  }
}
