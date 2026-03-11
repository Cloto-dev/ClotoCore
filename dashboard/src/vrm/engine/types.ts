export type AvatarAgentState = 'idle' | 'thinking' | 'responding';

/** Default pose bone adjustments (radians). MGP-compatible: set_idle_behavior can override. */
export interface DefaultPoseParams {
  // Upper arms: Z = lower from T-pose, Y = forward/back, X = twist
  left_upper_arm_z: number;
  right_upper_arm_z: number;
  left_upper_arm_y: number;
  right_upper_arm_y: number;
  left_upper_arm_x: number;
  right_upper_arm_x: number;
  // Lower arms: X = bend elbow, Z = fold inward/outward
  left_lower_arm_x: number;
  right_lower_arm_x: number;
  left_lower_arm_z: number;
  right_lower_arm_z: number;
  // Hands: Z rotation for natural wrist angle, X for flex/extend
  left_hand_z: number;
  right_hand_z: number;
  left_hand_x: number;
  right_hand_x: number;
  // Fingers: X rotation for curl (positive = curl inward)
  finger_spread: number;     // Z rotation for finger splay (thumb excluded)
  finger_curl_proximal: number;  // first knuckle
  finger_curl_intermediate: number; // second knuckle
  finger_curl_distal: number;    // fingertip
  thumb_curl_proximal: number;
  thumb_curl_distal: number;
  // Neck: X = pitch, Y = turn, Z = tilt
  neck_x: number;
  neck_y: number;
  neck_z: number;
  // Spine/head posture
  spine_x: number;
  spine_y: number;  // twist
  spine_z: number;  // side lean
  head_x: number;
  head_y: number;   // turn left/right
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
  left_upper_arm_y: 0,
  right_upper_arm_y: 0,
  left_upper_arm_x: 0.2,     // slight twist
  right_upper_arm_x: 0.2,
  left_lower_arm_x: 0.5,     // natural elbow bend
  right_lower_arm_x: 0.5,
  left_lower_arm_z: 0,
  right_lower_arm_z: 0,
  left_hand_z: -0.15,        // natural wrist angle
  right_hand_z: 0.15,
  left_hand_x: 0,
  right_hand_x: 0,
  finger_spread: 0.05,       // slight finger splay
  finger_curl_proximal: 0.35,   // ~20° relaxed first knuckle bend
  finger_curl_intermediate: 0.3,  // ~17° relaxed second knuckle
  finger_curl_distal: 0.2,     // ~11° relaxed fingertip curl
  thumb_curl_proximal: 0.3,    // relaxed thumb
  thumb_curl_distal: 0.2,
  neck_x: 0,
  neck_y: 0,
  neck_z: 0,
  spine_x: 0.02,             // very slight forward lean
  spine_y: 0,
  spine_z: 0,
  head_x: -0.03,             // slight chin-up
  head_y: 0,
  head_z: 0.02,              // tiny head tilt
};

export const DEFAULT_IDLE_PARAMS: IdleBehaviorParams = {
  mode: 'relaxed',
  breathing_rate: 1.0,
  sway_amplitude: 1.0,
  blink_frequency: 1.0,
  pose: DEFAULT_POSE,
};

// ── Preset Poses (humanoid-universal) ──

export const POSE_ATTENTIVE: DefaultPoseParams = {
  ...DEFAULT_POSE,
  left_upper_arm_x: 0.15,
  right_upper_arm_x: 0.15,
  left_lower_arm_x: 0.4,
  right_lower_arm_x: 0.4,
  spine_x: 0.05,             // forward lean — attentive
  head_x: 0.0,               // straight
  head_z: 0.0,               // no tilt
};

export const POSE_THINKING: DefaultPoseParams = {
  ...DEFAULT_POSE,
  left_upper_arm_z: 1.1,      // arm raised
  left_upper_arm_y: -1.3,     // forward
  left_upper_arm_x: 0.4,      // twist
  left_lower_arm_x: 2.3,      // deep elbow bend
  left_lower_arm_z: 2.0,      // fold forearm inward toward chin
  left_hand_z: -0.3,          // wrist angled for chin rest
  right_lower_arm_x: 0.2,     // relaxed right arm
  right_upper_arm_x: 0.05,
  finger_curl_proximal: 1.4,  // fingers curled (fist-like)
  finger_curl_intermediate: 1.2,
  finger_curl_distal: 1.3,
  finger_spread: 0.04,
  thumb_curl_proximal: 0.0,   // thumb relaxed
  thumb_curl_distal: 0.0,
  spine_x: -0.06,             // slight backward lean
  head_x: -0.02,
  head_z: 0.06,               // head tilt
};

export const POSE_ARMS_CROSSED: DefaultPoseParams = {
  ...DEFAULT_POSE,
  left_upper_arm_z: 0.8,     // more raised
  right_upper_arm_z: -0.8,
  left_upper_arm_x: 0.6,     // forward for crossing
  right_upper_arm_x: 0.6,
  left_lower_arm_x: 1.5,     // bent — arms across chest
  right_lower_arm_x: 1.5,
  left_hand_z: 0.3,          // wrist rotated inward
  right_hand_z: -0.3,
  finger_curl_proximal: 0.5, // tighter grip
  finger_curl_intermediate: 0.5,
  finger_curl_distal: 0.3,
  spine_x: 0.01,
  head_x: -0.02,
  head_z: 0.0,
};

/** All named pose presets. */
export const POSE_PRESETS: Record<string, DefaultPoseParams> = {
  relaxed: DEFAULT_POSE,
  attentive: POSE_ATTENTIVE,
  thinking: POSE_THINKING,
  arms_crossed: POSE_ARMS_CROSSED,
};
