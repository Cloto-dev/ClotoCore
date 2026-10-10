import { type VRM, type VRMHumanBoneName, type VRMHumanBones, VRMHumanoid } from '@pixiv/three-vrm';
import { Object3D, Vector3 } from 'three';
import { VrmExpressionMapper } from './VrmExpressionMapper';

export function motionRig(version: '0' | '1' = '1') {
  const scene = new Object3D();
  const nodes = new Map<VRMHumanBoneName, Object3D>();
  const add = (name: VRMHumanBoneName, parent: Object3D, x: number, y: number, z = 0) => {
    const node = new Object3D();
    node.name = name;
    node.position.set(x, y, z);
    parent.add(node);
    nodes.set(name, node);
    return node;
  };
  const hips = add('hips', scene, 0, 1, 0);
  const spine = add('spine', hips, 0, 0.15);
  const chest = add('chest', spine, 0, 0.2);
  const neck = add('neck', chest, 0, 0.22);
  add('head', neck, 0, 0.1);
  for (const side of ['left', 'right'] as const) {
    const sign = (side === 'left' ? 1 : -1) * (version === '1' ? 1 : -1);
    const shoulder = add(`${side}Shoulder`, chest, sign * 0.12, 0.1);
    const upper = add(`${side}UpperArm`, shoulder, sign * 0.07, 0);
    const lower = add(`${side}LowerArm`, upper, sign * 0.3, 0);
    const hand = add(`${side}Hand`, lower, sign * 0.27, 0);
    for (const [i, finger] of ['Index', 'Middle', 'Ring', 'Little'].entries()) {
      const proximal = add(`${side}${finger}Proximal` as VRMHumanBoneName, hand, sign * 0.055, 0, (1 - i) * 0.018);
      const intermediate = add(`${side}${finger}Intermediate` as VRMHumanBoneName, proximal, sign * 0.027, 0);
      add(`${side}${finger}Distal` as VRMHumanBoneName, intermediate, sign * 0.018, 0);
    }
    const thumb = add(`${side}ThumbMetacarpal`, hand, sign * 0.02, -0.008, 0.035);
    const thumbProximal = add(`${side}ThumbProximal`, thumb, sign * 0.025, 0, 0.012);
    add(`${side}ThumbDistal`, thumbProximal, sign * 0.018, 0);
    const leg = add(`${side}UpperLeg`, hips, sign * 0.1, -0.05);
    const shin = add(`${side}LowerLeg`, leg, 0, -0.45);
    add(`${side}Foot`, shin, 0, -0.45, -0.06);
  }
  scene.updateMatrixWorld(true);
  const humanoid = new VRMHumanoid(
    Object.fromEntries([...nodes].map(([name, node]) => [name, { node }])) as VRMHumanBones,
  );
  scene.add(humanoid.normalizedHumanBonesRoot);
  // Match the loader: build the normalized rig before rotateVRM0 rotates its scene.
  if (version === '0') scene.rotation.y = Math.PI;
  const expressions = new Map<string, number>();
  const vrm = {
    scene,
    humanoid,
    meta: { metaVersion: version },
    lookAt: { target: null },
    expressionManager: {
      getExpression: () => ({}),
      setValue: (name: string, value: number) => expressions.set(name, value),
    },
    update: () => humanoid.update(),
  } as unknown as VRM;
  const mapper = new VrmExpressionMapper();
  mapper.initialize(vrm);
  const position = (name: VRMHumanBoneName) => humanoid.getNormalizedBoneNode(name)!.getWorldPosition(new Vector3());
  return { vrm, expressions, mapper, position };
}
