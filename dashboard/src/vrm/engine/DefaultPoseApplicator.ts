import { VRM } from '@pixiv/three-vrm';
import { DefaultPoseParams, DEFAULT_POSE } from './types';

/**
 * Applies a default resting pose to VRM humanoid bones.
 * Called each frame after resetNormalizedPose() to establish the base pose
 * before procedural animation layers are applied additively on top.
 *
 * MGP-ready: params can be overridden via set_idle_behavior tool in future.
 */
export class DefaultPoseApplicator {
  private params: DefaultPoseParams = { ...DEFAULT_POSE };

  setParams(params: DefaultPoseParams) {
    this.params = { ...params };
  }

  apply(vrm: VRM) {
    const humanoid = vrm.humanoid;
    if (!humanoid) return;

    const p = this.params;

    // Upper arms — lower from T-pose
    const leftUpperArm = humanoid.getNormalizedBoneNode('leftUpperArm');
    if (leftUpperArm) {
      leftUpperArm.rotation.z += p.left_upper_arm_z;
      leftUpperArm.rotation.x += p.left_upper_arm_x;
    }

    const rightUpperArm = humanoid.getNormalizedBoneNode('rightUpperArm');
    if (rightUpperArm) {
      rightUpperArm.rotation.z += p.right_upper_arm_z;
      rightUpperArm.rotation.x += p.right_upper_arm_x;
    }

    // Lower arms — elbow bend
    const leftLowerArm = humanoid.getNormalizedBoneNode('leftLowerArm');
    if (leftLowerArm) {
      leftLowerArm.rotation.x += p.left_lower_arm_x;
    }

    const rightLowerArm = humanoid.getNormalizedBoneNode('rightLowerArm');
    if (rightLowerArm) {
      rightLowerArm.rotation.x += p.right_lower_arm_x;
    }

    // Hands — wrist angle
    const leftHand = humanoid.getNormalizedBoneNode('leftHand');
    if (leftHand) {
      leftHand.rotation.z += p.left_hand_z;
    }

    const rightHand = humanoid.getNormalizedBoneNode('rightHand');
    if (rightHand) {
      rightHand.rotation.z += p.right_hand_z;
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

    // Spine — slight forward lean
    const spine = humanoid.getNormalizedBoneNode('spine');
    if (spine) {
      spine.rotation.x += p.spine_x;
    }

    // Head — chin angle and tilt
    const head = humanoid.getNormalizedBoneNode('head');
    if (head) {
      head.rotation.x += p.head_x;
      head.rotation.z += p.head_z;
    }
  }
}
