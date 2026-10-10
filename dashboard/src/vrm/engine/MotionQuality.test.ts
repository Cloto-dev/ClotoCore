import { Quaternion, Vector3 } from 'three';
import { expect, it } from 'vitest';
import { CompanionMotion } from './CompanionMotion';
import { COMPANION_GESTURES, COMPANION_POSES, DEFAULT_COMPANION_IDLE } from './companionMotionLibrary';
import { motionRig } from './motionRig.test-utils';

const quiet = { ...DEFAULT_COMPANION_IDLE, breathing_rate: 0, sway_amplitude: 0, blink_frequency: 0 };
function setup(version: '0' | '1') {
  const rig = motionRig(version);
  const motion = new CompanionMotion(31);
  motion.setVrm(rig.vrm, rig.mapper);
  const frame = () => {
    rig.vrm.humanoid.resetNormalizedPose();
    motion.advance(1 / 60);
    motion.applyBase();
    motion.applyLife(1 / 60, quiet, new Vector3(0, 1.67, 1));
    rig.vrm.update(1 / 60);
  };
  return { ...rig, motion, frame };
}

it('keeps wrists and fingers continuous when every gesture is restarted or interrupted from every pose', () => {
  for (const version of ['0', '1'] as const)
    for (const pose of Object.keys(COMPANION_POSES))
      for (const first of COMPANION_GESTURES)
        for (const second of COMPANION_GESTURES) {
          const m = setup(version);
          m.motion.setPose(pose);
          for (let i = 0; i < 90; i++) m.frame();
          m.motion.playGesture(first);
          for (let i = 0; i < 72; i++) m.frame();
          m.motion.playGesture(second);
          const fingers = Object.entries(m.vrm.humanoid.normalizedHumanBones)
            .filter(([name]) => /Index|Middle|Ring|Little|Thumb/.test(name))
            .map(([, bone]) => bone!.node);
          for (let i = 0; i < 150; i++) {
            const hands = [m.position('leftHand'), m.position('rightHand')];
            const before = fingers.map((finger) => finger.quaternion.clone());
            const forearm = m.vrm.humanoid.getNormalizedBoneNode('rightLowerArm')!;
            const forearmBefore = forearm.quaternion.clone();
            m.frame();
            for (const [j, side] of (['left', 'right'] as const).entries())
              expect(
                m.position(`${side}Hand`).distanceTo(hands[j]),
                `${version}/${pose}/${first}->${second}/${i}`,
              ).toBeLessThan(i === 0 ? 0.04 : 0.08);
            expect(
              Math.max(...fingers.map((finger, index) => finger.quaternion.angleTo(before[index]))),
              `${first}->${second} fingers`,
            ).toBeLessThan(0.12);
            expect(forearm.quaternion.angleTo(forearmBefore), `${first}->${second} forearm roll`).toBeLessThan(0.075);
          }
        }
}, 15_000);

it('keeps folded wrists within the authored deflection limit and differentiates the thinking hand', () => {
  for (const version of ['0', '1'] as const) {
    const m = setup(version);
    m.motion.setPose('arms_crossed');
    for (let i = 0; i < 90; i++) m.frame();
    for (const side of ['left', 'right'] as const)
      expect(m.vrm.humanoid.getNormalizedBoneNode(`${side}Hand`)!.quaternion.angleTo(new Quaternion())).toBeLessThan(
        0.8,
      );
    m.motion.setPose('thinking');
    for (let i = 0; i < 90; i++) m.frame();
    const angle = (side: 'left' | 'right') =>
      m.vrm.humanoid.getNormalizedBoneNode(`${side}RingProximal`)!.quaternion.angleTo(new Quaternion());
    expect(angle('right') - angle('left')).toBeGreaterThan(0.25);
    expect(
      m.vrm.humanoid.getNormalizedBoneNode('rightLittleIntermediate')!.quaternion.angleTo(new Quaternion()),
    ).toBeGreaterThan(0.3);
  }
});

it('extends the stretch elbows and opens only the greeting hand before returning all finger joints', () => {
  const m = setup('1');
  m.motion.setPose('arms_crossed');
  for (let i = 0; i < 90; i++) m.frame();
  const left = m.vrm.humanoid.getNormalizedBoneNode('leftRingIntermediate')!.quaternion.clone();
  m.motion.playGesture('greet');
  for (let i = 0; i < 72; i++) m.frame();
  expect(m.vrm.humanoid.getNormalizedBoneNode('leftRingIntermediate')!.quaternion.angleTo(left)).toBeLessThan(0.001);
  expect(
    m.vrm.humanoid.getNormalizedBoneNode('rightRingIntermediate')!.quaternion.angleTo(new Quaternion()),
  ).toBeLessThan(0.1);
  for (let i = 0; i < 140; i++) m.frame();
  expect(
    m.vrm.humanoid.getNormalizedBoneNode('rightRingIntermediate')!.quaternion.angleTo(new Quaternion()),
  ).toBeGreaterThan(0.3);
  m.motion.playGesture('stretch');
  for (let i = 0; i < 156; i++) m.frame();
  for (const side of ['left', 'right'] as const) {
    const elbow = m.position(`${side}LowerArm`);
    const upper = m.position(`${side}UpperArm`).sub(elbow);
    const lower = m.position(`${side}Hand`).sub(elbow);
    expect(upper.angleTo(lower)).toBeGreaterThan(2.65);
  }
});

it('cups every folded finger toward the palm rather than leaving a flat or backwards grip', () => {
  for (const version of ['0', '1'] as const) {
    const m = setup(version);
    m.motion.setPose('arms_crossed');
    for (let i = 0; i < 90; i++) m.frame();
    for (const side of ['left', 'right'] as const) {
      const hand = m.vrm.humanoid.getNormalizedBoneNode(`${side}Hand`)!;
      const inverse = hand.getWorldQuaternion(new Quaternion()).invert();
      for (const finger of ['Index', 'Middle', 'Ring', 'Little'] as const) {
        const proximal = m.vrm.humanoid.getNormalizedBoneNode(`${side}${finger}Proximal`)!;
        const intermediate = m.vrm.humanoid.getNormalizedBoneNode(`${side}${finger}Intermediate`)!;
        const distal = m.vrm.humanoid.getNormalizedBoneNode(`${side}${finger}Distal`)!;
        const offset = distal
          .getWorldPosition(new Vector3())
          .sub(proximal.getWorldPosition(new Vector3()))
          .applyQuaternion(inverse);
        expect(offset.y, `${version}/${side}/${finger} bends into palm`).toBeLessThan(-0.025);
        expect(intermediate.quaternion.angleTo(new Quaternion())).toBeGreaterThan(0.75);
        const restDirection = intermediate.position.clone().normalize();
        expect(
          restDirection.applyQuaternion(distal.quaternion).y,
          `${version}/${side}/${finger} fingertip flexion`,
        ).toBeLessThan(-0.35);
      }
    }
  }
});
