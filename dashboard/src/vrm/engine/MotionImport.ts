import { VRMHumanBoneName as BoneName, type VRMHumanBoneName } from '@pixiv/three-vrm';
import { VRMAnimation } from '@pixiv/three-vrm-animation';
import {
  type AnimationClip,
  LoadingManager,
  type Object3D,
  Quaternion,
  QuaternionKeyframeTrack,
  Texture,
  TextureLoader,
  VectorKeyframeTrack,
} from 'three';
import { MOTION_MAX_BYTES, readGlbDocument } from '../../mate/assetFiles';

export type MotionImportProblem = 'format' | 'size' | 'skeleton' | 'invalid';
export class MotionImportError extends Error {
  constructor(public readonly code: MotionImportProblem) {
    super(code);
  }
}

function key(name: string) {
  return name
    .replace(/^.*:/, '')
    .replace(/^mixamorig/i, '')
    .replace(/[^a-z0-9]/gi, '')
    .toLowerCase();
}
const bones = new Map<string, VRMHumanBoneName>(Object.values(BoneName).map((name) => [key(name), name]));
for (const [alias, name] of Object.entries({
  Hips: 'hips',
  Spine: 'spine',
  Spine1: 'chest',
  Spine2: 'upperChest',
  Neck: 'neck',
  Head: 'head',
}))
  bones.set(key(alias), name as VRMHumanBoneName);
for (const side of ['Left', 'Right']) {
  const prefix = side.toLowerCase();
  for (const [alias, name] of Object.entries({
    Shoulder: 'Shoulder',
    Arm: 'UpperArm',
    ForeArm: 'LowerArm',
    Hand: 'Hand',
    UpLeg: 'UpperLeg',
    Leg: 'LowerLeg',
    Foot: 'Foot',
    ToeBase: 'Toes',
  }))
    bones.set(key(side + alias), (prefix + name) as VRMHumanBoneName);
  for (const finger of ['Thumb', 'Index', 'Middle', 'Ring', 'Pinky']) {
    const parts = finger === 'Thumb' ? ['Metacarpal', 'Proximal', 'Distal'] : ['Proximal', 'Intermediate', 'Distal'];
    parts.forEach((part, i) =>
      bones.set(
        key(`${side}Hand${finger}${i + 1}`),
        `${prefix}${finger === 'Pinky' ? 'Little' : finger}${part}` as VRMHumanBoneName,
      ),
    );
  }
}

/** Convert rest-relative, Y-up humanoid tracks into VRMA's normalized bone coordinates. */
export function retargetHumanoid(root: Object3D, clip: AnimationClip): VRMAnimation {
  root.updateMatrixWorld(true);
  const nodes = new Map<string, Object3D>();
  // FBX may append a second named control node after the animated skeleton.
  root.traverse((node) => {
    if (!nodes.has(node.name)) nodes.set(node.name, node);
  });
  const animation = new VRMAnimation();
  animation.duration = clip.duration;
  for (const track of clip.tracks) {
    const split = track.name.lastIndexOf('.');
    const nodeName = track.name.slice(0, split);
    const bone = bones.get(key(nodeName));
    const node = nodes.get(nodeName);
    if (!bone || !node) continue;
    if (
      Array.from(track.times).some((v) => !Number.isFinite(v) || v < 0) ||
      Array.from(track.values).some((v) => !Number.isFinite(v))
    )
      throw new MotionImportError('invalid');
    if (track instanceof QuaternionKeyframeTrack) {
      const inverse = node.getWorldQuaternion(new Quaternion()).invert();
      const parent = node.parent?.getWorldQuaternion(new Quaternion()) ?? new Quaternion();
      const values = track.values.slice();
      for (let i = 0; i < values.length; i += 4) {
        const q = new Quaternion().fromArray(values, i);
        if (q.lengthSq() < 0.001) throw new MotionImportError('invalid');
        q.normalize().premultiply(parent).multiply(inverse).normalize().toArray(values, i);
      }
      animation.humanoidTracks.rotation.set(
        bone,
        new QuaternionKeyframeTrack(`${bone}.quaternion`, track.times, values),
      );
    } else if (bone === 'hips' && track instanceof VectorKeyframeTrack && track.name.endsWith('.position')) {
      animation.humanoidTracks.translation.set('hips', track.clone());
      // Source units cancel against this height when createVRMAnimationClip scales translation.
      animation.restHipsPosition.set(0, node.position.y, 0);
    }
  }
  const required: VRMHumanBoneName[] = [
    'hips',
    'spine',
    'leftUpperArm',
    'rightUpperArm',
    'leftUpperLeg',
    'rightUpperLeg',
  ];
  if (required.some((name) => !animation.humanoidTracks.rotation.has(name))) throw new MotionImportError('skeleton');
  if (!Number.isFinite(clip.duration) || clip.duration <= 0 || clip.duration > 600)
    throw new MotionImportError('invalid');
  if (animation.humanoidTracks.translation.size && animation.restHipsPosition.y < 0.001)
    throw new MotionImportError('skeleton');
  return animation;
}

export async function importMotion(file: File, loadVrma: (file: File) => Promise<VRMAnimation>): Promise<VRMAnimation> {
  if (file.size > MOTION_MAX_BYTES) throw new MotionImportError('size');
  const extension = file.name.split('.').pop()?.toLowerCase();
  const bytes = await file.arrayBuffer();
  if (extension === 'vrma' || extension === 'glb') {
    readGlbDocument(bytes, 'motion');
    const animation = await loadVrma(file);
    if (
      !Number.isFinite(animation.duration) ||
      animation.duration <= 0 ||
      animation.duration > 600 ||
      !animation.humanoidTracks.rotation.size
    )
      throw new MotionImportError('invalid');
    for (const track of [
      ...animation.humanoidTracks.rotation.values(),
      ...animation.humanoidTracks.translation.values(),
    ]) {
      if (
        Array.from(track.times).some((v) => !Number.isFinite(v) || v < 0) ||
        Array.from(track.values).some((v) => !Number.isFinite(v))
      )
        throw new MotionImportError('invalid');
    }
    return animation;
  }
  if (extension === 'bvh') {
    const text = new TextDecoder().decode(bytes);
    // Bound the synchronous parser before building a skeleton or allocating frames.
    const frames = Number(/Frames:\s*(\d+)/i.exec(text)?.[1]);
    const step = Number(/Frame Time:\s*([\d.e+-]+)/i.exec(text)?.[1]);
    if (
      !/^\s*HIERARCHY\s+ROOT\s/.test(text) ||
      !Number.isFinite(frames) ||
      frames < 2 ||
      frames > 36000 ||
      !Number.isFinite(step) ||
      step <= 0 ||
      frames * step > 600 ||
      (text.match(/\bJOINT\b/g)?.length ?? 0) > 200
    )
      throw new MotionImportError('invalid');
    const { BVHLoader } = await import('three/examples/jsm/loaders/BVHLoader.js');
    const parsed = new BVHLoader().parse(text);
    return retargetHumanoid(parsed.skeleton.bones[0], parsed.clip);
  }
  if (extension === 'fbx') {
    const { FBXLoader } = await import('three/examples/jsm/loaders/FBXLoader.js');
    const manager = new LoadingManager();
    // Only animation data is used. Skip every texture load, including remote filenames.
    class NoTextures extends TextureLoader {
      override load() {
        return new Texture<HTMLImageElement>();
      }
    }
    manager.addHandler(/.*/, new NoTextures(manager));
    const root = new FBXLoader(manager).parse(bytes, '');
    try {
      const clips = root.animations.filter((clip) => clip.tracks.length > 0 && clip.duration > 0);
      if (clips.length !== 1) throw new MotionImportError('invalid');
      return retargetHumanoid(root, clips[0]);
    } finally {
      root.traverse((node) => {
        const mesh = node as Object3D & {
          geometry?: { dispose(): void };
          material?: { dispose(): void } | { dispose(): void }[];
        };
        mesh.geometry?.dispose();
        const materials = Array.isArray(mesh.material) ? mesh.material : mesh.material ? [mesh.material] : [];
        for (const material of materials) material.dispose();
      });
    }
  }
  throw new MotionImportError('format');
}
