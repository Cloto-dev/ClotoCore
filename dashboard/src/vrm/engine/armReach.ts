import type { VRM } from '@pixiv/three-vrm';
import { Quaternion, Vector3 } from 'three';

/** Two-bone reach in world space, using each avatar's actual limb lengths. */
export function reachHand(vrm: VRM, side: 'left' | 'right', target: Vector3, pole: Vector3, weight: number) {
  const upper = vrm.humanoid?.getNormalizedBoneNode(`${side}UpperArm`);
  const lower = vrm.humanoid?.getNormalizedBoneNode(`${side}LowerArm`);
  const hand = vrm.humanoid?.getNormalizedBoneNode(`${side}Hand`);
  if (!upper || !lower || !hand || weight <= 0) return;
  upper.updateWorldMatrix(true, true);
  const shoulder = upper.getWorldPosition(new Vector3());
  const elbow = lower.getWorldPosition(new Vector3());
  const wrist = hand.getWorldPosition(new Vector3());
  const a = shoulder.distanceTo(elbow);
  const b = elbow.distanceTo(wrist);
  if (a < 0.0001 || b < 0.0001) return;
  const direction = target.clone().sub(shoulder);
  const distance = Math.max(Math.abs(a - b) + 0.0001, Math.min(direction.length(), a + b - 0.0001));
  if (direction.lengthSq() < 1e-10) return;
  direction.normalize();
  const bend = pole.clone().sub(shoulder);
  bend.addScaledVector(direction, -bend.dot(direction));
  if (bend.lengthSq() < 1e-10) {
    bend.set(0, -1, 0).addScaledVector(direction, direction.y);
    if (bend.lengthSq() < 1e-10) bend.set(1, 0, 0);
  }
  bend.normalize();
  const along = (a * a + distance * distance - b * b) / (2 * distance);
  const desiredElbow = shoulder
    .clone()
    .addScaledVector(direction, along)
    .addScaledVector(bend, Math.sqrt(Math.max(0, a * a - along * along)));
  const desiredWrist = shoulder.clone().addScaledVector(direction, distance);
  function turn(node: NonNullable<typeof upper>, current: Vector3, desired: Vector3) {
    const before = node.quaternion.clone();
    const world = node.getWorldQuaternion(new Quaternion());
    world.premultiply(new Quaternion().setFromUnitVectors(current.normalize(), desired.normalize()));
    const parent = node.parent?.getWorldQuaternion(new Quaternion()) ?? new Quaternion();
    node.quaternion.copy(parent.invert().multiply(world));
    node.quaternion.slerpQuaternions(before, node.quaternion.clone(), Math.min(1, weight));
    node.updateWorldMatrix(false, true);
  }
  turn(upper, elbow.sub(shoulder), desiredElbow.clone().sub(shoulder));
  const actualElbow = lower.getWorldPosition(new Vector3());
  turn(lower, hand.getWorldPosition(new Vector3()).sub(actualElbow), desiredWrist.sub(actualElbow));
}

/** Orient the palm as well as the wrist; reaching alone leaves fingers pointing through the opposite arm. */
export function orientHand(vrm: VRM, side: 'left' | 'right', direction: Vector3, palm: Vector3, weight: number) {
  const hand = vrm.humanoid?.getNormalizedBoneNode(`${side}Hand`);
  const lower = vrm.humanoid?.getNormalizedBoneNode(`${side}LowerArm`);
  if (!hand || weight <= 0 || direction.lengthSq() < 1e-8) return;
  const before = hand.quaternion.clone();
  const world = hand.getWorldQuaternion(new Quaternion());
  const finger = vrm.humanoid.getNormalizedBoneNode(`${side}MiddleProximal`);
  const sign = (side === 'left' ? 1 : -1) * (vrm.meta.metaVersion === '1' ? 1 : -1);
  const current = finger
    ? finger.getWorldPosition(new Vector3()).sub(hand.getWorldPosition(new Vector3())).normalize()
    : new Vector3(sign, 0, 0).applyQuaternion(world);
  let aim = direction.clone().normalize();
  const forearm = lower
    ? hand.getWorldPosition(new Vector3()).sub(lower.getWorldPosition(new Vector3())).normalize()
    : current;
  const bend = forearm.angleTo(aim);
  if (bend > 0.65)
    aim = forearm
      .clone()
      .applyQuaternion(
        new Quaternion().slerpQuaternions(
          new Quaternion(),
          new Quaternion().setFromUnitVectors(forearm, aim),
          0.65 / bend,
        ),
      );
  world.premultiply(new Quaternion().setFromUnitVectors(current, aim));
  const normal = new Vector3(0, -1, 0).applyQuaternion(world);
  normal.addScaledVector(aim, -normal.dot(aim)).normalize();
  const desired = palm.clone().addScaledVector(aim, -palm.dot(aim)).normalize();
  if (normal.lengthSq() > 1e-8 && desired.lengthSq() > 1e-8) {
    const angle = Math.atan2(aim.dot(normal.clone().cross(desired)), normal.dot(desired));
    world.premultiply(new Quaternion().setFromAxisAngle(aim, angle));
  }
  const parent = hand.parent?.getWorldQuaternion(new Quaternion()) ?? new Quaternion();
  let local = parent.clone().invert().multiply(world);
  if (lower) {
    // Move palm roll into forearm pronation rather than twisting the wrist.
    const axis = hand.position.clone().normalize();
    const projection = new Vector3(local.x, local.y, local.z).dot(axis);
    const twist = new Quaternion(axis.x * projection, axis.y * projection, axis.z * projection, local.w).normalize();
    lower.quaternion.multiply(new Quaternion().slerpQuaternions(new Quaternion(), twist, Math.min(1, weight)));
    lower.updateWorldMatrix(false, true);
    local = lower.getWorldQuaternion(new Quaternion()).invert().multiply(world);
  }
  const angle = local.angleTo(new Quaternion());
  if (angle > 0.75) local = new Quaternion().slerp(local, 0.75 / angle);
  hand.quaternion.slerpQuaternions(before, local, Math.min(1, weight));
  hand.updateWorldMatrix(false, true);
}
