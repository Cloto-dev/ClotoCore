import type { DefaultPoseParams } from './engine/types';

interface SliderDef {
  key: keyof DefaultPoseParams;
  label: string;
  min: number;
  max: number;
  step: number;
}

export const SLIDER_GROUPS: { group: string; sliders: SliderDef[] }[] = [
  {
    group: 'Left Upper Arm',
    sliders: [
      { key: 'left_upper_arm_z', label: 'Z (lower)', min: -1.5, max: 2.0, step: 0.05 },
      { key: 'left_upper_arm_y', label: 'Y (fwd/back)', min: -2.0, max: 2.0, step: 0.05 },
      { key: 'left_upper_arm_x', label: 'X (twist)', min: -1.5, max: 1.5, step: 0.05 },
    ],
  },
  {
    group: 'Right Upper Arm',
    sliders: [
      { key: 'right_upper_arm_z', label: 'Z (lower)', min: -2.0, max: 1.5, step: 0.05 },
      { key: 'right_upper_arm_y', label: 'Y (fwd/back)', min: -2.0, max: 2.0, step: 0.05 },
      { key: 'right_upper_arm_x', label: 'X (twist)', min: -1.5, max: 1.5, step: 0.05 },
    ],
  },
  {
    group: 'Left Lower Arm',
    sliders: [
      { key: 'left_lower_arm_x', label: 'X (bend)', min: -1.0, max: 3.0, step: 0.05 },
      { key: 'left_lower_arm_z', label: 'Z (fold)', min: -2.0, max: 2.0, step: 0.05 },
    ],
  },
  {
    group: 'Right Lower Arm',
    sliders: [
      { key: 'right_lower_arm_x', label: 'X (bend)', min: -1.0, max: 3.0, step: 0.05 },
      { key: 'right_lower_arm_z', label: 'Z (fold)', min: -2.0, max: 2.0, step: 0.05 },
    ],
  },
  {
    group: 'Hands',
    sliders: [
      { key: 'left_hand_z', label: 'Left Z (angle)', min: -1.0, max: 1.0, step: 0.05 },
      { key: 'left_hand_x', label: 'Left X (flex)', min: -1.0, max: 1.0, step: 0.05 },
      { key: 'right_hand_z', label: 'Right Z (angle)', min: -1.0, max: 1.0, step: 0.05 },
      { key: 'right_hand_x', label: 'Right X (flex)', min: -1.0, max: 1.0, step: 0.05 },
    ],
  },
  {
    group: 'Fingers',
    sliders: [
      { key: 'finger_spread', label: 'Spread', min: 0, max: 0.3, step: 0.01 },
      { key: 'finger_curl_proximal', label: 'Curl 1st', min: 0, max: 1.5, step: 0.05 },
      { key: 'finger_curl_intermediate', label: 'Curl 2nd', min: 0, max: 1.5, step: 0.05 },
      { key: 'finger_curl_distal', label: 'Curl 3rd', min: 0, max: 1.5, step: 0.05 },
      { key: 'thumb_curl_proximal', label: 'Thumb 1st', min: 0, max: 1.0, step: 0.05 },
      { key: 'thumb_curl_distal', label: 'Thumb 2nd', min: 0, max: 1.0, step: 0.05 },
    ],
  },
  {
    group: 'Neck',
    sliders: [
      { key: 'neck_x', label: 'X (pitch)', min: -0.5, max: 0.5, step: 0.01 },
      { key: 'neck_y', label: 'Y (turn)', min: -0.5, max: 0.5, step: 0.01 },
      { key: 'neck_z', label: 'Z (tilt)', min: -0.3, max: 0.3, step: 0.01 },
    ],
  },
  {
    group: 'Spine',
    sliders: [
      { key: 'spine_x', label: 'X (lean)', min: -0.3, max: 0.3, step: 0.01 },
      { key: 'spine_y', label: 'Y (twist)', min: -0.5, max: 0.5, step: 0.01 },
      { key: 'spine_z', label: 'Z (side)', min: -0.3, max: 0.3, step: 0.01 },
    ],
  },
  {
    group: 'Head',
    sliders: [
      { key: 'head_x', label: 'X (nod)', min: -0.5, max: 0.5, step: 0.01 },
      { key: 'head_y', label: 'Y (turn)', min: -0.5, max: 0.5, step: 0.01 },
      { key: 'head_z', label: 'Z (tilt)', min: -0.3, max: 0.3, step: 0.01 },
    ],
  },
];
