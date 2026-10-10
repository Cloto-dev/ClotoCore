import { VRMAnimation } from '@pixiv/three-vrm-animation';
import { AnimationClip, Bone, Object3D, Quaternion, QuaternionKeyframeTrack, VectorKeyframeTrack } from 'three';
import { describe, expect, it, vi } from 'vitest';
import { importMotion, retargetHumanoid } from './MotionImport';

function sourceRig() {
  const root = new Object3D();
  root.rotation.y = 0.4;
  const nodes = ['Hips', 'Spine', 'LeftArm', 'RightArm', 'LeftUpLeg', 'RightUpLeg', 'LeftHandIndex1'].map((name) => {
    const node = new Bone();
    node.name = `mixamorig${name}`;
    node.rotation.z = 0.2;
    root.add(node);
    return node;
  });
  nodes[0].position.y = 100;
  const duplicateControl = new Object3D();
  duplicateControl.name = nodes[0].name;
  root.add(duplicateControl);
  const tracks = nodes.map(
    (node) =>
      new QuaternionKeyframeTrack(
        `${node.name}.quaternion`,
        [0, 1],
        [...node.quaternion.toArray(), ...node.quaternion.toArray()],
      ),
  );
  return { root, nodes, tracks };
}

function bvhFile() {
  const names = ['Spine', 'LeftArm', 'RightArm', 'LeftUpLeg', 'RightUpLeg'];
  const text = `HIERARCHY\nROOT Hips\n{\nOFFSET 0 100 0\nCHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation\n${names.map((name) => `JOINT ${name}\n{\nOFFSET 0 5 0\nCHANNELS 3 Zrotation Xrotation Yrotation\nEnd Site\n{\nOFFSET 0 5 0\n}\n}`).join('\n')}\n}\nMOTION\nFrames: 2\nFrame Time: 0.5\n${Array(21).fill('0').join(' ')}\n${['10', ...Array(20).fill('0')].join(' ')}\n`;
  return {
    name: 'probe.bvh',
    size: text.length,
    arrayBuffer: async () => new TextEncoder().encode(text).buffer,
  } as File;
}

describe('motion import', () => {
  it('ignores empty FBX takes but rejects multiple playable clips', async () => {
    const { FBXLoader } = await import('three/examples/jsm/loaders/FBXLoader.js');
    const { root, tracks } = sourceRig();
    const clip = new AnimationClip('body', 1, tracks);
    root.animations = [clip, new AnimationClip('empty', 0, [])];
    const parse = vi.spyOn(FBXLoader.prototype, 'parse').mockReturnValue(root as any);
    const file = { name: 'probe.fbx', size: 1, arrayBuffer: async () => new ArrayBuffer(1) } as File;
    try {
      expect(
        (
          await importMotion(file, async () => {
            throw new Error('Wrong loader');
          })
        ).humanoidTracks.rotation.size,
      ).toBe(7);
      root.animations.push(clip.clone());
      await expect(
        importMotion(file, async () => {
          throw new Error('Wrong loader');
        }),
      ).rejects.toThrow('invalid');
    } finally {
      parse.mockRestore();
    }
  });
  it('removes source rest rotations, maps fingers and preserves source hip units', () => {
    const { root, tracks } = sourceRig();
    tracks.push(new VectorKeyframeTrack('mixamorigHips.position', [0, 1], [0, 100, 0, 10, 100, 0]) as any);
    const converted = retargetHumanoid(root, new AnimationClip('source', 1, tracks));
    expect(converted.restHipsPosition.y).toBe(100);
    expect(converted.humanoidTracks.translation.get('hips')?.values[3]).toBe(10);
    const q = new Quaternion().fromArray(converted.humanoidTracks.rotation.get('leftIndexProximal')!.values);
    expect(q.angleTo(new Quaternion())).toBeLessThan(0.00001);
    expect(converted.humanoidTracks.rotation.size).toBe(7);
  });
  it('parses a BVH through the real loader and retains motion displacement', async () => {
    const animation = await importMotion(bvhFile(), async () => {
      throw new Error('Wrong loader');
    });
    expect(animation.duration).toBe(0.5);
    expect(animation.humanoidTracks.translation.get('hips')?.values[3]).toBe(10);
    expect(animation.humanoidTracks.rotation.has('leftUpperArm')).toBe(true);
  });
  it('rejects unknown skeletons rather than silently returning a partial body', () => {
    const { root, tracks } = sourceRig();
    root.children[2].name = 'unmappedLeftArm';
    expect(() => retargetHumanoid(root, new AnimationClip('source', 1, tracks))).toThrow('skeleton');
  });
  it('rejects nonfinite curves and NPZ output', async () => {
    const { root, tracks } = sourceRig();
    tracks[0].values[0] = Number.NaN;
    expect(() => retargetHumanoid(root, new AnimationClip('source', 1, tracks))).toThrow('invalid');
    await expect(
      importMotion({ name: 'motion.npz', size: 1, arrayBuffer: async () => new ArrayBuffer(1) } as File, async () => {
        throw new Error('Wrong loader');
      }),
    ).rejects.toThrow('format');
  });
  it('rejects nonfinite VRMA curves before registering them', async () => {
    const json = new TextEncoder().encode(JSON.stringify({ extensions: { VRMC_vrm_animation: {} } }).padEnd(64, ' '));
    const bytes = new ArrayBuffer(20 + json.length);
    const view = new DataView(bytes);
    view.setUint32(0, 0x46546c67, true);
    view.setUint32(4, 2, true);
    view.setUint32(8, bytes.byteLength, true);
    view.setUint32(12, json.length, true);
    view.setUint32(16, 0x4e4f534a, true);
    new Uint8Array(bytes, 20).set(json);
    const file = { name: 'invalid.vrma', size: bytes.byteLength, arrayBuffer: async () => bytes } as File;
    const motion = new VRMAnimation();
    motion.duration = 1;
    motion.humanoidTracks.rotation.set(
      'hips',
      new QuaternionKeyframeTrack('hips.quaternion', [0, 1], [0, 0, 0, 1, Number.NaN, 0, 0, 1]),
    );
    await expect(importMotion(file, async () => motion)).rejects.toThrow('invalid');
  });
});
