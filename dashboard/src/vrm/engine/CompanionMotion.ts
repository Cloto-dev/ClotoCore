import type { VRM, VRMHumanBoneName } from '@pixiv/three-vrm';
import { Euler, Object3D, Quaternion, Vector3 } from 'three';
import { orientHand, reachHand } from './armReach';
import {
  COMPANION_POSES,
  COMPANION_REST,
  type CompanionGesture,
  motionRandom,
  smoothMotion,
} from './companionMotionLibrary';
import type { AvatarAgentState, DefaultPoseParams, IdleBehaviorParams } from './types';
import type { VrmExpressionMapper } from './VrmExpressionMapper';

type BoneRotations = Map<VRMHumanBoneName, Quaternion>;
const durations: Record<CompanionGesture, number> = { greet: 3.2, nod: 1.8, look_around: 4.6, stretch: 5.2 };
const bounded = (value: number, low: number, high: number) => Math.max(low, Math.min(high, value));
const rate = (value: number) => (Number.isFinite(value) ? bounded(value, 0, 2) : 0);

/** A coordinated, deterministic motion layer. Feet and authored bone scales remain untouched. */
export class CompanionMotion {
  private vrm: VRM | null = null;
  private mapper: VrmExpressionMapper | null = null;
  private time = 0;
  private random: () => number;
  private pose: BoneRotations = new Map();
  private from: BoneRotations = new Map();
  private to: BoneRotations = new Map();
  private poseElapsed = 1;
  private poseDuration = 0.85;
  private reach = { thinking: 0, crossed: 0 };
  private reachFrom = { thinking: 0, crossed: 0 };
  private reachTo = { thinking: 0, crossed: 0 };
  private state: AvatarAgentState = 'idle';
  private thought = 0;
  private speaking = 0;
  private gesture: { name: CompanionGesture; start: number } | null = null;
  private nextGlance = 12;
  private nextNod = 7;
  private nextBlink = 3;
  private blinkStart: number | null = null;
  private blinkClock = 0;
  private doubleBlink = false;
  private eyes = new Object3D();
  private gazeYaw = 0;
  private gazePitch = 0;
  private eyeInitialized = false;
  private lastOutput: BoneRotations = new Map();
  private interruption: BoneRotations = new Map();
  private interruptionStart = -1;

  constructor(seed = 74021) {
    this.random = motionRandom(seed);
  }

  setVrm(vrm: VRM, mapper: VrmExpressionMapper) {
    this.vrm = vrm;
    this.mapper = mapper;
    this.pose = this.rotations(COMPANION_REST);
    this.from = new Map(this.pose);
    this.to = new Map(this.pose);
    this.eyeInitialized = false;
  }

  setState(state: AvatarAgentState) {
    this.state = state;
  }

  setPose(name: string, duration = 0.85) {
    const params = COMPANION_POSES[name];
    if (!params) return;
    this.setPoseParams(params, duration);
    this.reachTo = { thinking: name === 'thinking' ? 1 : 0, crossed: name === 'arms_crossed' ? 1 : 0 };
  }

  setPoseParams(params: DefaultPoseParams, duration = 0.3) {
    this.captureInterruption();
    this.from = new Map([...this.pose].map(([name, q]) => [name, q.clone()]));
    this.to = this.rotations(params);
    this.reachFrom = { ...this.reach };
    this.reachTo = { thinking: 0, crossed: 0 };
    this.poseElapsed = 0;
    this.poseDuration = Math.max(0.01, duration);
  }

  playGesture(name: CompanionGesture) {
    if (this.gesture) this.captureInterruption();
    this.gesture = { name, start: this.time };
  }

  private captureInterruption() {
    this.interruption = new Map([...this.lastOutput].map(([name, q]) => [name, q.clone()]));
    this.interruptionStart = this.time;
  }

  private finishFrame(dt: number) {
    const humanoid = this.vrm?.humanoid;
    if (!humanoid) return;
    const weight = smoothMotion((this.time - this.interruptionStart) / 0.7);
    if (weight < 1)
      for (const [name, from] of this.interruption) {
        const node = humanoid.getNormalizedBoneNode(name);
        if (node) node.quaternion.slerpQuaternions(from, node.quaternion.clone(), weight);
      }
    // Bound joint speed through intermediate IK configurations, including changes in forearm roll.
    for (const [name, previous] of this.lastOutput) {
      const node = humanoid.getNormalizedBoneNode(name);
      if (!node) continue;
      const angle = previous.angleTo(node.quaternion);
      const maximum = Math.max(0, dt) * (/Index|Middle|Ring|Little|Thumb/.test(name) ? 6 : 4);
      if (angle > maximum) node.quaternion.slerpQuaternions(previous, node.quaternion.clone(), maximum / angle);
    }
    this.rememberRenderedPose();
  }

  rememberRenderedPose() {
    const humanoid = this.vrm?.humanoid;
    if (!humanoid) return;
    this.lastOutput.clear();
    for (const [name, bone] of Object.entries(humanoid.normalizedHumanBones))
      if (bone) this.lastOutput.set(name as VRMHumanBoneName, bone.node.quaternion.clone());
  }

  /** Advance only animation time; called exactly once per rendered frame. */
  advance(delta: number) {
    const dt = Math.max(0, delta);
    this.time += dt;
    this.blinkClock += dt;
    this.poseElapsed += dt;
    const mix = smoothMotion(this.poseElapsed / this.poseDuration);
    for (const [name, q] of this.to) {
      this.pose.set(name, new Quaternion().slerpQuaternions(this.from.get(name) ?? new Quaternion(), q, mix));
    }
    this.reach.thinking = this.reachFrom.thinking + (this.reachTo.thinking - this.reachFrom.thinking) * mix;
    this.reach.crossed = this.reachFrom.crossed + (this.reachTo.crossed - this.reachFrom.crossed) * mix;
    const damping = 1 - Math.exp(-3.2 * dt);
    this.thought += ((this.state === 'thinking' ? 1 : 0) - this.thought) * damping;
    this.speaking += ((this.state === 'responding' ? 1 : 0) - this.speaking) * damping;
  }

  applyBase() {
    if (!this.vrm?.humanoid) return;
    for (const [name, quaternion] of this.pose)
      this.vrm.humanoid.getNormalizedBoneNode(name)?.quaternion.copy(quaternion);
  }

  applyLife(
    dt: number,
    params: IdleBehaviorParams,
    gaze: Vector3,
    reducedMotion = false,
    manualMotion: boolean | 'pose' = false,
  ) {
    const vrm = this.vrm;
    if (!vrm) return;
    // An imported animation owns all body motion; no idle rotations are added to its joints.
    if (manualMotion) {
      if (manualMotion !== 'pose') {
        this.blinkStart = null;
        this.nextBlink = this.blinkClock + 3;
      }
      this.nextGlance = this.time + 12;
      this.nextNod = this.time + 7;
      this.gesture = null;
      this.interruption.clear();
      this.lastOutput.clear();
      if (manualMotion === 'pose') {
        // A frozen imported pose owns joints, while eyes and eyelids stay alive.
        this.updateGaze(dt, gaze, true, 0);
        this.updateBlink(params.blink_frequency, 0);
      }
      return;
    }
    const breathing = reducedMotion ? 0 : rate(params.breathing_rate);
    const sway = reducedMotion ? 0 : rate(params.sway_amplitude);
    if (!reducedMotion && sway > 0 && !this.gesture) {
      if (this.state === 'responding' && this.time >= this.nextNod) {
        this.playGesture('nod');
        this.nextNod = this.time + 7 + this.random() * 6;
      } else if (this.state === 'idle' && this.time >= this.nextGlance) {
        this.playGesture('look_around');
        this.nextGlance = this.time + 15 + this.random() * 12;
      }
    }
    const g = this.gesture;
    const phase = g ? (this.time - g.start) / durations[g.name] : 1;
    const heldGesture = g?.name === 'greet' || g?.name === 'stretch';
    const envelope =
      !g || phase >= 1
        ? 0
        : heldGesture
          ? Math.min(smoothMotion(phase / 0.22), smoothMotion((1 - phase) / 0.22))
          : Math.sin(Math.PI * phase) ** 2;
    if (g && phase >= 1) this.gesture = null;
    const calm = params.mode === 'sleepy' ? 0.55 : params.mode === 'attentive' ? 0.75 : 1;
    const breathPhase = ((this.time * breathing) / 5.2) % 1;
    // Shorter inhale, longer release, with zero speed at either end.
    const breath =
      breathPhase < 0.38 ? smoothMotion(breathPhase / 0.38) : 1 - smoothMotion((breathPhase - 0.38) / 0.62);
    this.rotate('spine', -0.004 * breath * breathing, 0, 0.007 * Math.sin(this.time * 0.46) * sway * calm);
    this.rotate('chest', -0.008 * breath * breathing, 0.009 * Math.sin(this.time * 0.31 + 0.6) * sway * calm, 0);
    this.rotate('leftShoulder', 0, 0, -0.005 * breath * breathing);
    this.rotate('rightShoulder', 0, 0, 0.005 * breath * breathing);
    this.rotate('leftUpperArm', 0.012 * Math.sin(this.time * 0.49 + 0.7) * sway, 0, 0);
    this.rotate('rightUpperArm', 0.009 * Math.sin(this.time * 0.43 + 2.1) * sway, 0, 0);
    this.rotate(
      'head',
      0.012 * this.thought,
      -0.045 * this.thought,
      0.025 * this.thought + 0.007 * Math.sin(this.time * 0.36 + 1.4) * sway * calm,
    );
    this.rotate('spine', 0.018 * this.speaking, 0, 0);
    if (g?.name === 'nod') this.rotate('head', 0.14 * envelope, 0, 0);
    if (g?.name === 'look_around')
      this.rotate('head', 0, 0.24 * Math.sin(phase * Math.PI * 2) * envelope, -0.02 * envelope);
    if (g?.name === 'stretch') this.rotate('chest', -0.065 * envelope, 0, 0);
    for (const side of ['left', 'right'] as const) {
      const thinking = side === 'right' ? Math.max(this.reach.thinking, this.thought * (1 - this.reach.crossed)) : 0;
      this.shapeHand(side, 0.88, 0.8, 0.4, thinking);
      if (g?.name === 'stretch') this.shapeHand(side, 0.24, 0.18, 0.1, envelope);
    }
    if (g?.name === 'greet') this.openGreetingHand(envelope);
    this.applyReach(g?.name, envelope);
    this.updateGaze(dt, gaze, reducedMotion, envelope);
    this.updateBlink(params.blink_frequency, params.mode === 'sleepy' ? 0.12 : this.thought * 0.045);
    this.finishFrame(dt);
  }

  private rotations(params: DefaultPoseParams): BoneRotations {
    const result: BoneRotations = new Map();
    const put = (name: VRMHumanBoneName, x = 0, y = 0, z = 0) => {
      const sign = this.vrm?.meta.metaVersion === '1' ? -1 : 1;
      result.set(name, new Quaternion().setFromEuler(new Euler(x * sign, y, z * sign)));
    };
    for (const side of ['left', 'right'] as const) {
      put(
        `${side}UpperArm`,
        params[`${side}_upper_arm_x`],
        params[`${side}_upper_arm_y`],
        params[`${side}_upper_arm_z`],
      );
      put(`${side}LowerArm`, params[`${side}_lower_arm_x`], 0, params[`${side}_lower_arm_z`]);
      put(`${side}Hand`, params[`${side}_hand_x`], 0, params[`${side}_hand_z`]);
      const sign = side === 'left' ? 1 : -1;
      for (const [i, finger] of ['Index', 'Middle', 'Ring', 'Little'].entries()) {
        put(
          `${side}${finger}Proximal` as VRMHumanBoneName,
          0,
          sign * (i - 1) * params.finger_spread,
          sign * params.finger_curl_proximal * (0.88 + i * 0.08),
        );
        put(`${side}${finger}Intermediate` as VRMHumanBoneName, 0, 0, sign * params.finger_curl_intermediate);
        put(`${side}${finger}Distal` as VRMHumanBoneName, 0, 0, sign * params.finger_curl_distal);
      }
      put(
        `${side}ThumbMetacarpal`,
        params.thumb_curl_proximal * 0.3,
        sign * params.thumb_curl_proximal * 0.9,
        sign * params.thumb_curl_proximal,
      );
      put(`${side}ThumbProximal`, 0, 0, sign * params.thumb_curl_proximal * 0.45);
      put(`${side}ThumbDistal`, 0, 0, sign * params.thumb_curl_distal);
    }
    put('neck', params.neck_x, params.neck_y, params.neck_z);
    put('spine', params.spine_x, params.spine_y, params.spine_z);
    put('head', params.head_x, params.head_y, params.head_z);
    return result;
  }

  private rotate(name: VRMHumanBoneName, x: number, y: number, z: number) {
    const node = this.vrm?.humanoid?.getNormalizedBoneNode(name);
    if (!node) return;
    const sign = this.vrm?.meta.metaVersion === '1' ? -1 : 1;
    node.quaternion.multiply(new Quaternion().setFromEuler(new Euler(sign * x, y, sign * z)));
  }

  private openGreetingHand(weight: number) {
    const open = this.rotations({
      ...COMPANION_REST,
      finger_curl_proximal: 0.05,
      finger_curl_intermediate: 0.025,
      finger_curl_distal: 0.02,
      thumb_curl_proximal: 0.08,
      thumb_curl_distal: 0.04,
    });
    for (const [name, rotation] of open) {
      if (name.startsWith('right') && /Index|Middle|Ring|Little|Thumb/.test(name))
        this.vrm?.humanoid.getNormalizedBoneNode(name)?.quaternion.slerp(rotation, weight);
    }
  }

  private shapeHand(side: 'left' | 'right', proximal: number, intermediate: number, distal: number, weight: number) {
    if (weight <= 0) return;
    const shape = this.rotations({
      ...COMPANION_REST,
      finger_curl_proximal: proximal,
      finger_curl_intermediate: intermediate,
      finger_curl_distal: distal,
      finger_spread: 0.015,
      thumb_curl_proximal: proximal * 0.55,
      thumb_curl_distal: distal * 0.8,
    });
    for (const [name, q] of shape)
      if (name.startsWith(side) && /Index|Middle|Ring|Little|Thumb/.test(name))
        this.vrm?.humanoid.getNormalizedBoneNode(name)?.quaternion.slerp(q, weight);
  }

  private applyReach(gesture: CompanionGesture | undefined, envelope: number) {
    const vrm = this.vrm;
    const humanoid = vrm?.humanoid;
    const headNode = humanoid?.getNormalizedBoneNode('head');
    const hipsNode = humanoid?.getNormalizedBoneNode('hips');
    const left = humanoid?.getNormalizedBoneNode('leftUpperArm');
    const right = humanoid?.getNormalizedBoneNode('rightUpperArm');
    if (!vrm || !headNode || !hipsNode || !left || !right) return;
    const head = headNode.getWorldPosition(new Vector3());
    const hips = hipsNode.getWorldPosition(new Vector3());
    const leftPos = left.getWorldPosition(new Vector3());
    const rightPos = right.getWorldPosition(new Vector3());
    const height = head.distanceTo(hips);
    if (height < 0.05) return;
    const up = head.clone().sub(hips).normalize();
    const sideAxis = leftPos.clone().sub(rightPos).normalize();
    const forward = sideAxis.clone().cross(up).normalize();
    const point = (origin: Vector3, side: number, vertical: number, front: number) =>
      origin
        .clone()
        .addScaledVector(sideAxis, height * side)
        .addScaledVector(up, height * vertical)
        .addScaledVector(forward, height * front);
    const chest =
      humanoid?.getNormalizedBoneNode('chest')?.getWorldPosition(new Vector3()) ?? hips.clone().lerp(head, 0.64);
    for (const side of ['left', 'right'] as const) {
      const sign = side === 'left' ? 1 : -1;
      const shoulder = side === 'left' ? leftPos : rightPos;
      const armGesture = gesture === 'stretch' || (gesture === 'greet' && side === 'right');
      const retainedPose = armGesture ? 1 - envelope : 1;
      const crossWeight = this.reach.crossed * retainedPose;
      const thinkWeight =
        side === 'right' ? Math.max(this.reach.thinking, this.thought * (1 - this.reach.crossed)) * retainedPose : 0;
      const weight = crossWeight + thinkWeight;
      if (weight > 0.001) {
        // The lower forearm supports the other elbow; the upper hand rests beside the opposite upper arm.
        // Separate height and depth keep the wrists and fingers out of one another and in front of the torso.
        const cross = side === 'left' ? point(chest, -0.22, 0.13, 0.27) : point(chest, 0.22, 0.25, 0.25);
        const chin = point(head, -0.1, -0.25, 0.25);
        const target = cross.multiplyScalar(crossWeight).addScaledVector(chin, thinkWeight).divideScalar(weight);
        reachHand(vrm, side, target, point(shoulder, sign * 0.36, -0.48, 0.25), weight);
        if (crossWeight > 0.001) {
          const direction = sideAxis
            .clone()
            .multiplyScalar(-sign)
            .addScaledVector(up, side === 'left' ? 0.35 : -0.55);
          orientHand(vrm, side, direction, forward.clone().negate(), crossWeight);
        }
        if (thinkWeight > 0.001)
          orientHand(vrm, side, up.clone().addScaledVector(sideAxis, 0.12), forward.clone().negate(), thinkWeight);
      }
      if (gesture === 'greet' && side === 'right') {
        reachHand(vrm, side, point(head, -0.65, -0.02, 0.24), point(shoulder, -0.42, -0.15, 0.18), envelope);
        const wave = 0.2 * Math.sin((this.time - (this.gesture?.start ?? this.time)) * 8);
        orientHand(vrm, side, up.clone().addScaledVector(sideAxis, 0.12 + wave), forward, envelope);
      }
      if (gesture === 'stretch') {
        const lower = humanoid?.getNormalizedBoneNode(`${side}LowerArm`);
        const hand = humanoid?.getNormalizedBoneNode(`${side}Hand`);
        const length =
          lower && hand
            ? shoulder.distanceTo(lower.getWorldPosition(new Vector3())) +
              lower.getWorldPosition(new Vector3()).distanceTo(hand.getWorldPosition(new Vector3()))
            : height;
        const direction = up
          .clone()
          .addScaledVector(sideAxis, sign * 0.28)
          .addScaledVector(forward, 0.05)
          .normalize();
        reachHand(
          vrm,
          side,
          shoulder.clone().addScaledVector(direction, length * 0.98),
          point(shoulder, sign * 0.55, 0.4, 0.12),
          envelope,
        );
        orientHand(vrm, side, direction, forward, envelope);
      }
    }
  }

  private updateGaze(dt: number, target: Vector3, reduced: boolean, gestureWeight: number) {
    const vrm = this.vrm;
    const head = vrm?.humanoid?.getNormalizedBoneNode('head');
    if (!vrm || !head) return;
    const position = head.getWorldPosition(new Vector3());
    const offset = target.clone().sub(position);
    const yaw = bounded(Math.atan2(offset.x, Math.max(0.3, offset.z)), -0.38, 0.38);
    const pitch = bounded(-Math.atan2(offset.y, Math.max(0.3, offset.z)), -0.18, 0.18);
    const blend = 1 - Math.exp(-3 * dt);
    this.gazeYaw += (yaw - this.gazeYaw) * blend;
    this.gazePitch += (pitch - this.gazePitch) * blend;
    if (!reduced) {
      this.rotate('neck', this.gazePitch * 0.14, this.gazeYaw * 0.18 * (1 - gestureWeight), 0);
      this.rotate('head', this.gazePitch * 0.3, this.gazeYaw * 0.4 * (1 - gestureWeight), 0);
    }
    if (vrm.lookAt) {
      const glance =
        this.gesture?.name === 'look_around'
          ? 0.24 * Math.sin(((this.time - this.gesture.start) / durations.look_around) * Math.PI * 2) * gestureWeight
          : 0;
      const limited = position
        .clone()
        .add(new Vector3(Math.tan(yaw * (1 - gestureWeight) + glance), -Math.tan(pitch), 1));
      if (!this.eyeInitialized) {
        this.eyes.position.copy(limited);
        this.eyeInitialized = true;
      }
      this.eyes.position.lerp(limited, 1 - Math.exp(-12 * dt));
      vrm.lookAt.target = this.eyes;
    }
  }

  private updateBlink(frequency: number, restingLid: number) {
    const multiplier = rate(frequency);
    let value = 0;
    if (multiplier === 0) {
      this.blinkStart = null;
      this.nextBlink = this.blinkClock + 3;
    } else {
      if (this.blinkStart === null && this.blinkClock >= this.nextBlink) {
        this.blinkStart = this.nextBlink;
        this.doubleBlink = this.random() < 0.12;
      }
      if (this.blinkStart !== null) {
        const elapsed = this.blinkClock - this.blinkStart;
        const duration = this.doubleBlink ? 0.58 : 0.26;
        const local = elapsed > 0.32 && this.doubleBlink ? elapsed - 0.32 : elapsed;
        if (local < 0.075) value = smoothMotion(local / 0.075);
        else if (local < 0.1) value = 1;
        else if (local < 0.26) value = 1 - smoothMotion((local - 0.1) / 0.16);
        if (elapsed >= duration) {
          this.blinkStart = null;
          this.nextBlink = this.blinkStartTimeEnd(elapsed, duration) + (2.8 + this.random() * 3.6) / multiplier;
        }
      }
    }
    // A single owner combines blink and resting eyelids; thinking never overwrites a full blink.
    const combined = Math.max(value, restingLid);
    for (const name of this.mapper?.getBlinkNames() ?? []) this.vrm?.expressionManager?.setValue(name, combined);
  }

  private blinkStartTimeEnd(elapsed: number, duration: number) {
    return this.blinkClock - elapsed + duration;
  }
}
