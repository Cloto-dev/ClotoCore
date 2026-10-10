import { type VRM, VRMHumanoid } from '@pixiv/three-vrm';
import { Object3D, Vector3 } from 'three';
import { describe, expect, it } from 'vitest';
import { DefaultPoseApplicator } from './DefaultPoseApplicator';

function rig(version: '0' | '1') {
  const hips = new Object3D();
  const left = new Object3D();
  const elbow = new Object3D();
  const hand = new Object3D();
  const sign = version === '1' ? 1 : -1;
  hips.position.y = 1;
  hips.add(left);
  left.position.set(sign * 0.2, 0.4, 0);
  left.add(elbow);
  elbow.position.x = sign * 0.3;
  elbow.add(hand);
  hand.position.x = sign * 0.25;
  const humanoid = new VRMHumanoid({
    head: { node: new Object3D() },
    spine: { node: new Object3D() },
    leftUpperLeg: { node: new Object3D() },
    leftLowerLeg: { node: new Object3D() },
    leftFoot: { node: new Object3D() },
    rightUpperLeg: { node: new Object3D() },
    rightLowerLeg: { node: new Object3D() },
    rightFoot: { node: new Object3D() },
    rightUpperArm: { node: new Object3D() },
    rightLowerArm: { node: new Object3D() },
    rightHand: { node: new Object3D() },
    hips: { node: hips },
    leftUpperArm: { node: left },
    leftLowerArm: { node: elbow },
    leftHand: { node: hand },
  });
  const vrm = { humanoid, meta: { metaVersion: version } } as unknown as VRM;
  const shoulder = humanoid.getNormalizedBoneNode('leftUpperArm')!;
  const wrist = humanoid.getNormalizedBoneNode('leftHand')!;
  const before = shoulder.getWorldPosition(new Vector3()).y;
  new DefaultPoseApplicator().apply(vrm);
  return { before, after: wrist.getWorldPosition(new Vector3()).y };
}
describe('resting pose format orientation', () => {
  it('lowers the hand below the shoulder for a VRM 1 normalized rig', () => {
    const pose = rig('1');
    expect(pose.after).toBeLessThan(pose.before - 0.4);
  });
  it('preserves the resting pose for a VRM 0 normalized rig', () => {
    const pose = rig('0');
    expect(pose.after).toBeLessThan(pose.before - 0.4);
    expect(pose.after).toBeCloseTo(rig('1').after, 5);
  });
});
