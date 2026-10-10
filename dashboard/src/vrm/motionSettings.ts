import type { AgentMetadata } from '../types';
import { COMPANION_POSES, DEFAULT_COMPANION_IDLE, type MotionStyle } from './engine/companionMotionLibrary';
import {
  DEFAULT_IDLE_PARAMS,
  DEFAULT_POSE,
  type DefaultPoseParams,
  type IdleBehaviorParams,
  POSE_PRESETS,
} from './engine/types';
import type { VrmAnimationController } from './engine/VrmAnimationController';

export function savedMotionStyle(agent: AgentMetadata): MotionStyle {
  return agent.metadata?.mate_motion_style === 'legacy' ? 'legacy' : 'companion';
}

export function motionKeys(style: MotionStyle) {
  return style === 'legacy'
    ? { idle: 'mate_idle', pose: 'mate_pose' }
    : { idle: 'mate_companion_idle', pose: 'mate_companion_pose' };
}

export function motionPoses(style: MotionStyle) {
  return style === 'legacy' ? POSE_PRESETS : COMPANION_POSES;
}

export function savedMotionPose(agent: AgentMetadata, style = savedMotionStyle(agent)) {
  const name = agent.metadata?.[motionKeys(style).pose] ?? '';
  return Object.hasOwn(motionPoses(style), name) ? name : '';
}

export function savedIdleBehavior(agent: AgentMetadata, style = savedMotionStyle(agent)): IdleBehaviorParams {
  const fallback = style === 'legacy' ? DEFAULT_IDLE_PARAMS : DEFAULT_COMPANION_IDLE;
  const defaults = () => ({ ...fallback, pose: { ...fallback.pose } });
  try {
    const parsed = JSON.parse(agent.metadata?.[motionKeys(style).idle] ?? 'null');
    if (!parsed || !['relaxed', 'attentive', 'sleepy'].includes(parsed.mode)) return defaults();
    const pose = { ...DEFAULT_POSE };
    for (const key of Object.keys(pose) as (keyof DefaultPoseParams)[]) {
      const value = parsed.pose?.[key];
      if (typeof value !== 'number' || !Number.isFinite(value)) return defaults();
      pose[key] = value;
    }
    const rates = ['breathing_rate', 'sway_amplitude', 'blink_frequency'] as const;
    if (
      rates.some(
        (key) => typeof parsed[key] !== 'number' || !Number.isFinite(parsed[key]) || parsed[key] < 0 || parsed[key] > 2,
      )
    )
      return defaults();
    return {
      mode: parsed.mode,
      breathing_rate: parsed.breathing_rate,
      sway_amplitude: parsed.sway_amplitude,
      blink_frequency: parsed.blink_frequency,
      pose,
    };
  } catch {
    return defaults();
  }
}

/** Shared by the room and separate viewer, including settings refreshes from another window. */
export function applySavedMotion(agent: AgentMetadata, controller: VrmAnimationController) {
  const style = savedMotionStyle(agent);
  controller.setMotionStyle(style);
  controller.setIdleParams(savedIdleBehavior(agent, style));
  const preset = savedMotionPose(agent, style);
  if (preset) void controller.setPose(preset);
}
