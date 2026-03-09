export type AvatarAgentState = 'idle' | 'thinking' | 'responding';

/** Default pose bone adjustments (radians). MGP-compatible: set_idle_behavior can override. */
export interface DefaultPoseParams {
  // Upper arms: negative Z = lower from T-pose. Positive X = forward.
  left_upper_arm_z: number;
  right_upper_arm_z: number;
  left_upper_arm_x: number;
  right_upper_arm_x: number;
  // Lower arms: positive X = bend elbow inward
  left_lower_arm_x: number;
  right_lower_arm_x: number;
  // Hands: Z rotation for natural wrist angle
  left_hand_z: number;
  right_hand_z: number;
  // Fingers: X rotation for curl (positive = curl inward)
  finger_spread: number;     // Z rotation for finger splay (thumb excluded)
  finger_curl_proximal: number;  // first knuckle
  finger_curl_intermediate: number; // second knuckle
  finger_curl_distal: number;    // fingertip
  thumb_curl_proximal: number;
  thumb_curl_distal: number;
  // Spine/head posture
  spine_x: number;
  head_x: number;
  head_z: number;
}

export interface IdleBehaviorParams {
  mode: 'relaxed' | 'attentive' | 'sleepy';
  breathing_rate: number;   // 0.0-2.0 multiplier
  sway_amplitude: number;   // 0.0-2.0 multiplier
  blink_frequency: number;  // 0.0-2.0 multiplier
  pose: DefaultPoseParams;
}

export const DEFAULT_POSE: DefaultPoseParams = {
  left_upper_arm_z: 1.2,     // ~69° lower from T-pose (arms at sides)
  right_upper_arm_z: -1.2,
  left_upper_arm_x: 0.2,     // slightly forward
  right_upper_arm_x: 0.2,
  left_lower_arm_x: 0.5,     // natural elbow bend
  right_lower_arm_x: 0.5,
  left_hand_z: -0.15,        // natural wrist angle
  right_hand_z: 0.15,
  finger_spread: 0.05,       // slight finger splay
  finger_curl_proximal: 0.35,   // ~20° relaxed first knuckle bend
  finger_curl_intermediate: 0.3,  // ~17° relaxed second knuckle
  finger_curl_distal: 0.2,     // ~11° relaxed fingertip curl
  thumb_curl_proximal: 0.3,    // relaxed thumb
  thumb_curl_distal: 0.2,
  spine_x: 0.02,             // very slight forward lean
  head_x: -0.03,             // slight chin-up
  head_z: 0.02,              // tiny head tilt
};

export const DEFAULT_IDLE_PARAMS: IdleBehaviorParams = {
  mode: 'relaxed',
  breathing_rate: 1.0,
  sway_amplitude: 1.0,
  blink_frequency: 1.0,
  pose: DEFAULT_POSE,
};
