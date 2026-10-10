import { VRMAnimation } from '@pixiv/three-vrm-animation';
import { Quaternion, QuaternionKeyframeTrack, Vector3 } from 'three';
import { afterEach, expect, it, vi } from 'vitest';
import { motionRig } from './motionRig.test-utils';
import { VrmAnimationController } from './VrmAnimationController';
import type { VrmSceneManager } from './VrmSceneManager';

const m = vi.hoisted(() => ({ parse: vi.fn() }));
vi.mock('./MotionImport', () => ({ importMotion: m.parse }));
let controller: VrmAnimationController;
afterEach(() => {
  controller?.dispose();
  vi.resetAllMocks();
});
function setup() {
  const rig = motionRig();
  controller = new VrmAnimationController({
    mouseTarget: new Vector3(0, 1.6, 1),
    render: vi.fn(),
  } as unknown as VrmSceneManager);
  controller.setVrm(rig.vrm);
  return rig;
}
function animation() {
  const animation = new VRMAnimation();
  animation.duration = 2;
  const rotation = new Quaternion().setFromAxisAngle(new Vector3(1, 0, 0), 0.4);
  animation.humanoidTracks.rotation.set(
    'spine',
    new QuaternionKeyframeTrack('spine.quaternion', [0, 2], [...rotation.toArray(), ...rotation.toArray()]),
  );
  return { animation, rotation };
}
it('keeps an authored paused pose intact in legacy mode and retains playback identity across panel mounts', async () => {
  const rig = setup();
  const authored = animation();
  m.parse.mockResolvedValue(authored.animation);
  controller.setMotionStyle('legacy');
  await controller.loadComparisonMotion(new File(['data'], 'probe.bvh'));
  controller.pauseMotion(true);
  controller.seekMotion(1);
  for (let i = 0; i < 30; i++) controller.updateFrame(1 / 60);
  expect(rig.vrm.humanoid.getNormalizedBoneNode('spine')!.quaternion.angleTo(authored.rotation)).toBeLessThan(1e-6);
  expect(controller.motionPlayback.time).toBe(1);
  expect(controller.comparisonMotionName).toBe('probe.bvh');
  controller.stopVrma();
  expect(controller.comparisonMotionName).toBeNull();
});
it('rejects a completed import after stop and keeps a playing result when another import fails', async () => {
  setup();
  let resolve!: (a: VRMAnimation) => void;
  m.parse.mockReturnValue(
    new Promise((done) => {
      resolve = done;
    }),
  );
  const pending = controller.loadComparisonMotion(new File(['data'], 'old.bvh'));
  controller.stopVrma();
  resolve(animation().animation);
  await expect(pending).rejects.toThrow('selection changed');
  expect(controller.motionPlayback.active).toBe(false);
  m.parse.mockResolvedValue(animation().animation);
  await controller.loadComparisonMotion(new File(['data'], 'valid.bvh'));
  m.parse.mockRejectedValue(new Error('Invalid'));
  await expect(controller.loadComparisonMotion(new File(['data'], 'bad.npz'))).rejects.toThrow('Invalid');
  expect(controller.motionPlayback.active).toBe(true);
  expect(controller.comparisonMotionName).toBe('valid.bvh');
});
