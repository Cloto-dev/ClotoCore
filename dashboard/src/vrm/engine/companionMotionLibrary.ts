import type { DefaultPoseParams, IdleBehaviorParams } from './types';

export type MotionStyle = 'companion' | 'legacy';
export type CompanionGesture = 'greet' | 'nod' | 'look_around' | 'stretch';
export const COMPANION_GESTURES: CompanionGesture[] = ['greet', 'nod', 'look_around', 'stretch'];

/** Authored separately from the original presets, which remain available unchanged. */
export const COMPANION_REST: DefaultPoseParams = {
  left_upper_arm_z: 1.38,
  right_upper_arm_z: -1.38,
  left_upper_arm_y: -0.04,
  right_upper_arm_y: 0.04,
  left_upper_arm_x: 0.06,
  right_upper_arm_x: 0.04,
  left_lower_arm_x: 0.18,
  right_lower_arm_x: 0.14,
  left_lower_arm_z: 0.02,
  right_lower_arm_z: -0.02,
  left_hand_z: -0.05,
  right_hand_z: 0.05,
  left_hand_x: 0.02,
  right_hand_x: 0.01,
  finger_spread: 0.035,
  finger_curl_proximal: 0.24,
  finger_curl_intermediate: 0.18,
  finger_curl_distal: 0.12,
  thumb_curl_proximal: 0.18,
  thumb_curl_distal: 0.1,
  neck_x: 0,
  neck_y: 0,
  neck_z: 0,
  spine_x: 0.008,
  spine_y: 0,
  spine_z: 0,
  head_x: -0.012,
  head_y: 0,
  head_z: 0,
};

export const COMPANION_POSES: Record<string, DefaultPoseParams> = {
  relaxed: COMPANION_REST,
  attentive: {
    ...COMPANION_REST,
    spine_x: 0.055,
    head_x: -0.04,
    head_z: 0.018,
    left_upper_arm_z: 1.48,
    right_upper_arm_z: -1.48,
    left_lower_arm_z: -0.12,
    right_lower_arm_z: 0.12,
    left_lower_arm_x: 0.25,
    right_lower_arm_x: 0.22,
  },
  thinking: {
    ...COMPANION_REST,
    head_y: -0.1,
    head_z: 0.045,
    head_x: 0.035,
    right_lower_arm_x: 0.75,
    right_upper_arm_x: 0.2,
  },
  arms_crossed: {
    ...COMPANION_REST,
    left_upper_arm_z: 1.05,
    right_upper_arm_z: -1.05,
    left_upper_arm_x: 0.36,
    right_upper_arm_x: 0.32,
    left_lower_arm_x: 1.3,
    right_lower_arm_x: 1.25,
    finger_spread: 0.012,
    finger_curl_proximal: 0.6,
    finger_curl_intermediate: 0.95,
    finger_curl_distal: 0.45,
    thumb_curl_proximal: 0.48,
    thumb_curl_distal: 0.3,
    head_z: -0.02,
  },
};

export const DEFAULT_COMPANION_IDLE: IdleBehaviorParams = {
  mode: 'relaxed',
  breathing_rate: 1,
  sway_amplitude: 1,
  blink_frequency: 1,
  pose: COMPANION_REST,
};

export const smoothMotion = (t: number) => {
  const x = Math.max(0, Math.min(1, t));
  return x * x * x * (x * (x * 6 - 15) + 10);
};

/** Local reproducible scheduling; never consumes the application's global random stream. */
export function motionRandom(seed: number) {
  let value = seed >>> 0;
  return () => {
    value = (value + 0x6d2b79f5) >>> 0;
    let n = Math.imul(value ^ (value >>> 15), value | 1);
    n ^= n + Math.imul(n ^ (n >>> 7), n | 61);
    return ((n ^ (n >>> 14)) >>> 0) / 4294967296;
  };
}
