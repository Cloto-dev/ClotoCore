import type { VRM } from '@pixiv/three-vrm';
import { DEFAULT_POSE, type DefaultPoseParams } from './types';

/** Lerp a single number. */
function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

/** Lerp all fields of DefaultPoseParams. */
function lerpPose(a: DefaultPoseParams, b: DefaultPoseParams, t: number): DefaultPoseParams {
  const result = {} as DefaultPoseParams;
  for (const key of Object.keys(a) as (keyof DefaultPoseParams)[]) {
    result[key] = lerp(a[key], b[key], t);
  }
  return result;
}

/**
 * Applies a default resting pose to VRM humanoid bones.
 * Called each frame after resetNormalizedPose() to establish the base pose
 * before procedural animation layers are applied additively on top.
 *
 * Supports smooth transitions between poses via lerp.
 */
export class DefaultPoseApplicator {
  private current: DefaultPoseParams = { ...DEFAULT_POSE };
  private from: DefaultPoseParams = { ...DEFAULT_POSE };
  private target: DefaultPoseParams = { ...DEFAULT_POSE };
  private transitionProgress = 1.0; // 1.0 = no transition active
  private transitionDuration = 0.5; // seconds

  setParams(params: DefaultPoseParams) {
    this.current = { ...params };
    this.target = { ...params };
    this.transitionProgress = 1.0;
  }

  /** Transition smoothly to a new pose over the given duration (seconds). */
  transitionTo(params: DefaultPoseParams, durationSec = 0.5) {
    this.from = { ...this.current };
    this.target = { ...params };
    this.transitionDuration = Math.max(durationSec, 0.01);
    this.transitionProgress = 0.0;
  }

  /** Advance the transition by deltaTime. Call before apply(). */
  update(deltaTime: number) {
    if (this.transitionProgress >= 1.0) return;

    this.transitionProgress = Math.min(1.0, this.transitionProgress + deltaTime / this.transitionDuration);
    // Smoothstep easing for natural motion
    const t = this.transitionProgress;
    const ease = t * t * (3 - 2 * t);
    this.current = lerpPose(this.from, this.target, ease);
  }

  apply(vrm: VRM) {
    const humanoid = vrm.humanoid;
    if (!humanoid) return;

    const p = this.current;

    // Upper arms — Z: lower from T-pose, Y: forward/back, X: twist
    const leftUpperArm = humanoid.getNormalizedBoneNode('leftUpperArm');
    if (leftUpperArm) {
      leftUpperArm.rotation.z += p.left_upper_arm_z;
      leftUpperArm.rotation.y += p.left_upper_arm_y;
      leftUpperArm.rotation.x += p.left_upper_arm_x;
    }

    const rightUpperArm = humanoid.getNormalizedBoneNode('rightUpperArm');
    if (rightUpperArm) {
      rightUpperArm.rotation.z += p.right_upper_arm_z;
      rightUpperArm.rotation.y += p.right_upper_arm_y;
      rightUpperArm.rotation.x += p.right_upper_arm_x;
    }

    // Lower arms — elbow bend + fold
    const leftLowerArm = humanoid.getNormalizedBoneNode('leftLowerArm');
    if (leftLowerArm) {
      leftLowerArm.rotation.x += p.left_lower_arm_x;
      leftLowerArm.rotation.z += p.left_lower_arm_z;
    }

    const rightLowerArm = humanoid.getNormalizedBoneNode('rightLowerArm');
    if (rightLowerArm) {
      rightLowerArm.rotation.x += p.right_lower_arm_x;
      rightLowerArm.rotation.z += p.right_lower_arm_z;
    }

    // Hands — wrist angle + flex
    const leftHand = humanoid.getNormalizedBoneNode('leftHand');
    if (leftHand) {
      leftHand.rotation.z += p.left_hand_z;
      leftHand.rotation.x += p.left_hand_x;
    }

    const rightHand = humanoid.getNormalizedBoneNode('rightHand');
    if (rightHand) {
      rightHand.rotation.z += p.right_hand_z;
      rightHand.rotation.x += p.right_hand_x;
    }

    // Fingers — normalized bones, curl via Z axis
    // In VRM normalized T-pose, fingers extend along ±X axis.
    // Curl (toward palm) = Z rotation: left hand +Z, right hand -Z
    const fingerNames = ['Index', 'Middle', 'Ring', 'Little'] as const;
    for (const side of ['left', 'right'] as const) {
      const curlSign = side === 'left' ? 1 : -1;
      for (const finger of fingerNames) {
        const proximal = humanoid.getNormalizedBoneNode(`${side}${finger}Proximal` as any);
        if (proximal) {
          proximal.rotation.z += curlSign * p.finger_curl_proximal;
        }
        const intermediate = humanoid.getNormalizedBoneNode(`${side}${finger}Intermediate` as any);
        if (intermediate) {
          intermediate.rotation.z += curlSign * p.finger_curl_intermediate;
        }
        const distal = humanoid.getNormalizedBoneNode(`${side}${finger}Distal` as any);
        if (distal) {
          distal.rotation.z += curlSign * p.finger_curl_distal;
        }
      }
      // Thumb
      const thumbMeta = humanoid.getNormalizedBoneNode(`${side}ThumbMetacarpal` as any);
      if (thumbMeta) {
        thumbMeta.rotation.z += curlSign * p.thumb_curl_proximal;
      }
      const thumbProx = humanoid.getNormalizedBoneNode(`${side}ThumbProximal` as any);
      if (thumbProx) {
        thumbProx.rotation.z += curlSign * p.thumb_curl_proximal * 0.5;
      }
      const thumbDistal = humanoid.getNormalizedBoneNode(`${side}ThumbDistal` as any);
      if (thumbDistal) {
        thumbDistal.rotation.z += curlSign * p.thumb_curl_distal;
      }
    }

    // Neck
    const neck = humanoid.getNormalizedBoneNode('neck');
    if (neck) {
      neck.rotation.x += p.neck_x;
      neck.rotation.y += p.neck_y;
      neck.rotation.z += p.neck_z;
    }

    // Spine — lean, twist, side
    const spine = humanoid.getNormalizedBoneNode('spine');
    if (spine) {
      spine.rotation.x += p.spine_x;
      spine.rotation.y += p.spine_y;
      spine.rotation.z += p.spine_z;
    }

    // Head — pitch, turn, tilt
    const head = humanoid.getNormalizedBoneNode('head');
    if (head) {
      head.rotation.x += p.head_x;
      head.rotation.y += p.head_y;
      head.rotation.z += p.head_z;
    }
  }
}
