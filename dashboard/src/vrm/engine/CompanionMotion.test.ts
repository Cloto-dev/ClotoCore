import { Quaternion, Vector3 } from 'three';
import { expect, it } from 'vitest';
import { reachHand } from './armReach';
import { CompanionMotion } from './CompanionMotion';
import { DEFAULT_COMPANION_IDLE } from './companionMotionLibrary';
import { motionRig } from './motionRig.test-utils';

const quiet = { ...DEFAULT_COMPANION_IDLE, breathing_rate: 0, sway_amplitude: 0, blink_frequency: 0 };
function setup(version: '0' | '1' = '1') {
  const rig = motionRig(version);
  const motion = new CompanionMotion(31);
  motion.setVrm(rig.vrm, rig.mapper);
  const frame = (dt: number, params = quiet, manual = false) => {
    rig.vrm.humanoid.resetNormalizedPose();
    motion.advance(dt);
    motion.applyBase();
    motion.applyLife(dt, params, rig.position('head').add(new Vector3(0, 0, 1)), false, manual);
    rig.vrm.update(dt);
  };
  return { ...rig, motion, frame };
}

it('keeps the feet planted and the authored raw bone scale while rebuilding idle motion', () => {
  const m = setup();
  const foot = m.position('leftFoot');
  const chest = m.vrm.humanoid.getRawBoneNode('chest')!;
  chest.scale.set(1.1, 0.9, 1.05);
  for (let i = 0; i < 1200; i++) m.frame(1 / 60, DEFAULT_COMPANION_IDLE);
  expect(m.position('leftFoot').distanceTo(foot)).toBeLessThan(1e-8);
  expect(chest.scale.toArray()).toEqual([1.1, 0.9, 1.05]);
  m.motion.playGesture('stretch');
  m.frame(2.6);
  for (const side of ['left', 'right'] as const) {
    const height = m.position(`${side}Hand`).y - m.position('head').y;
    expect(height).toBeGreaterThan(0.08);
    const length =
      m.position(`${side}UpperArm`).distanceTo(m.position(`${side}LowerArm`)) +
      m.position(`${side}LowerArm`).distanceTo(m.position(`${side}Hand`));
    expect(height).toBeLessThan(length);
  }
  expect(m.position('leftFoot').distanceTo(foot)).toBeLessThan(1e-8);
});

it('lowers the resting hands in both VRM formats and raises a greeting once before returning', () => {
  for (const version of ['0', '1'] as const) {
    const m = setup(version);
    m.frame(1 / 60);
    const rest = m.position('rightHand');
    expect(rest.y).toBeLessThan(m.position('rightUpperArm').y - 0.5);
    m.motion.playGesture('greet');
    m.frame(1.1);
    expect(m.position('rightHand').y).toBeGreaterThan(rest.y + 0.4);
    const hand = m.vrm.humanoid.getNormalizedBoneNode('rightHand')!;
    expect(
      m.vrm.humanoid.getNormalizedBoneNode('rightIndexProximal')!.quaternion.angleTo(new Quaternion()),
    ).toBeLessThan(0.1);
    const firstWave = hand.quaternion.clone();
    m.frame(0.2);
    expect(hand.quaternion.angleTo(firstWave)).toBeGreaterThan(0.08);
    m.frame(2);
    expect(m.position('rightHand').distanceTo(rest)).toBeLessThan(0.00001);
  }
});

it('reaches a target using actual limb lengths and stays finite when the target is unreachable', () => {
  const m = setup();
  m.frame(1 / 60);
  const target = new Vector3(-0.33, 1.42, 0.21);
  reachHand(m.vrm, 'right', target, new Vector3(-0.5, 1.1, 0.2), 1);
  expect(m.position('rightHand').distanceTo(target)).toBeLessThan(0.001);
  reachHand(m.vrm, 'right', new Vector3(100, 100, 100), new Vector3(-1, 0, 0), 1);
  expect(m.position('rightHand').toArray().every(Number.isFinite)).toBe(true);
  expect(m.position('rightHand').distanceTo(m.position('rightUpperArm'))).toBeLessThan(0.571);
});

it('stacks folded forearms with hands beside the opposite upper arm and fingers continuing across the torso', () => {
  for (const version of ['0', '1'] as const) {
    const m = setup(version);
    m.motion.setPose('arms_crossed');
    m.frame(1.3);
    const left = m.position('leftHand');
    const right = m.position('rightHand');
    const chest = m.position('chest');
    expect(right.y - left.y).toBeGreaterThan(0.05);
    expect(left.z - chest.z).toBeGreaterThan(0.1);
    expect(right.z - chest.z).toBeGreaterThan(0.1);
    expect(right.distanceTo(m.position('leftUpperArm'))).toBeLessThan(0.3);
    const hand = m.vrm.humanoid.getNormalizedBoneNode('rightHand')!;
    const sign = version === '1' ? -1 : 1;
    const fingers = new Vector3(sign, 0, 0).applyQuaternion(hand.getWorldQuaternion(new Quaternion()));
    expect(fingers.x).toBeGreaterThan(0.5);
    expect(fingers.y).toBeLessThan(0.5);
    m.motion.playGesture('greet');
    m.frame(1.2);
    expect(m.position('leftHand').distanceTo(left)).toBeLessThan(0.002);
    m.frame(2.1);
    m.motion.playGesture('nod');
    m.frame(0.9);
    expect(m.position('leftHand').distanceTo(left)).toBeLessThan(0.002);
  }
});

it('keeps full blinks visible while thinking and stops automatic blinking at zero', () => {
  const m = setup();
  m.motion.setState('thinking');
  let maximum = 0;
  for (let i = 0; i < 480; i++) {
    m.frame(1 / 120, DEFAULT_COMPANION_IDLE);
    maximum = Math.max(maximum, m.expressions.get('blink') ?? 0);
  }
  expect(maximum).toBeGreaterThan(0.98);
  m.motion.setState('idle');
  for (let i = 0; i < 3000; i++) m.frame(1 / 120);
  expect(m.expressions.get('blink')).toBeLessThan(0.00001);
});

it('disables breathing and sway at zero and leaves imported animation joints alone', () => {
  const m = setup();
  m.frame(1 / 60);
  const spine = m.vrm.humanoid.getNormalizedBoneNode('spine')!;
  const rest = spine.quaternion.clone();
  for (let i = 0; i < 600; i++) m.frame(1 / 60);
  expect(spine.quaternion.angleTo(rest)).toBeLessThan(1e-6);
  const manual = new Quaternion().setFromAxisAngle(new Vector3(1, 0, 0), 0.42);
  spine.quaternion.copy(manual);
  m.motion.setState('responding');
  m.motion.advance(1);
  m.motion.applyLife(1 / 60, DEFAULT_COMPANION_IDLE, new Vector3(0, 1, 1), false, true);
  expect(spine.quaternion.angleTo(manual)).toBeLessThan(1e-6);
});

it('produces equivalent body poses at 30 and 120 Hz and smoothly interrupts a pose transition', () => {
  const sample = (hz: number) => {
    const m = setup();
    m.motion.setPose('thinking');
    m.motion.setState('thinking');
    for (let i = 0; i < hz * 2; i++) m.frame(1 / hz, DEFAULT_COMPANION_IDLE);
    return m;
  };
  const a = sample(30);
  const b = sample(120);
  expect(a.position('rightHand').distanceTo(b.position('rightHand'))).toBeLessThan(0.005);
  expect(
    a.vrm.humanoid
      .getNormalizedBoneNode('spine')!
      .quaternion.angleTo(b.vrm.humanoid.getNormalizedBoneNode('spine')!.quaternion),
  ).toBeLessThan(0.0005);
  const head = b.vrm.humanoid.getNormalizedBoneNode('head')!;
  const before = head.quaternion.clone();
  b.motion.setPose('attentive');
  b.frame(1 / 120, DEFAULT_COMPANION_IDLE);
  expect(head.quaternion.angleTo(before)).toBeLessThan(0.025);
});
