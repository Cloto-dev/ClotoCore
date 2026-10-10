import { VRMAnimation } from '@pixiv/three-vrm-animation';
import { Quaternion, QuaternionKeyframeTrack, Vector3, VectorKeyframeTrack } from 'three';
import { afterEach, expect, it, vi } from 'vitest';
import { motionRig } from './motionRig.test-utils';
import { VrmAnimationController } from './VrmAnimationController';
import { VrmaLoader } from './VrmaLoader';
import type { VrmSceneManager } from './VrmSceneManager';

const controllers: VrmAnimationController[] = [];
afterEach(() => {
  for (const controller of controllers.splice(0)) controller.dispose();
  vi.restoreAllMocks();
});
function setup() {
  const rig = motionRig();
  const render = vi.fn();
  const controller = new VrmAnimationController({
    mouseTarget: new Vector3(0, 1.67, 1),
    render,
  } as unknown as VrmSceneManager);
  controllers.push(controller);
  controller.setVrm(rig.vrm);
  return { ...rig, controller, render };
}
function animation() {
  const result = new VRMAnimation();
  result.duration = 2;
  result.restHipsPosition.set(0, 1, 0);
  const q = new Quaternion().setFromAxisAngle(new Vector3(1, 0, 0), 0.42);
  result.humanoidTracks.rotation.set(
    'spine',
    new QuaternionKeyframeTrack('spine.quaternion', [0, 2], [...q.toArray(), ...q.toArray()]),
  );
  return result;
}

it('uses the rebuilt state motion by default and retains the original thinking asset in legacy mode', async () => {
  const load = vi.spyOn(VrmaLoader.prototype, 'load').mockResolvedValue(animation());
  const m = setup();
  m.controller.setAgentState('thinking');
  for (let i = 0; i < 120; i++) m.controller.updateFrame(1 / 60);
  expect(load).not.toHaveBeenCalled();
  expect(m.position('rightHand').y).toBeGreaterThan(1.3);
  m.controller.setMotionStyle('legacy');
  await m.controller.setPose('thinking');
  expect(load).toHaveBeenCalledWith('/vrma/thinking.vrma');
});

it('rejects an original preset that finishes loading after switching to new movement', async () => {
  let resolve!: (value: VRMAnimation) => void;
  vi.spyOn(VrmaLoader.prototype, 'load').mockImplementation(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  const apply = vi.spyOn(VrmaLoader.prototype, 'applyPose');
  const m = setup();
  m.controller.setMotionStyle('legacy');
  const pending = m.controller.setPose('thinking');
  m.controller.setMotionStyle('companion');
  resolve(animation());
  await pending;
  expect(apply).not.toHaveBeenCalled();
  for (let i = 0; i < 90; i++) m.controller.updateFrame(1 / 60);
  expect(m.position('rightHand').y).toBeLessThan(1.1);
});

it('rejects a delayed original preset after a newer posture was chosen in the same style', async () => {
  let resolve!: (value: VRMAnimation) => void;
  vi.spyOn(VrmaLoader.prototype, 'load').mockImplementation(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  const apply = vi.spyOn(VrmaLoader.prototype, 'applyPose');
  const m = setup();
  m.controller.setMotionStyle('legacy');
  const pending = m.controller.setPose('thinking');
  await m.controller.setPose('attentive');
  resolve(animation());
  await pending;
  expect(apply).not.toHaveBeenCalled();
});

it('leaves manually loaded VRMA joints unchanged by thinking, idle motion or gesture previews', async () => {
  vi.spyOn(VrmaLoader.prototype, 'loadFile').mockResolvedValue(animation());
  const m = setup();
  m.expressions.set('blink', 1);
  await m.controller.loadVrmaAnimationFile(new File(['fixture'], 'manual.vrma'));
  m.controller.setAgentState('thinking');
  m.controller.previewGesture('greet');
  for (let i = 0; i < 120; i++) m.controller.updateFrame(1 / 60);
  const expected = new Quaternion().setFromAxisAngle(new Vector3(1, 0, 0), 0.42);
  expect(m.vrm.humanoid.getNormalizedBoneNode('spine')!.quaternion.angleTo(expected)).toBeLessThan(1e-6);
  expect(m.controller.isVrmaActive).toBe(true);
  expect(m.expressions.get('blink')).toBe(0);
  expect(m.vrm.lookAt?.target).toBeNull();
  expect(m.render).toHaveBeenCalledTimes(120);
});

it('restores the avatar chest scale when leaving original movement', () => {
  const m = motionRig();
  const chest = m.vrm.humanoid.getRawBoneNode('chest')!;
  chest.scale.set(1.1, 0.9, 1.05);
  const controller = new VrmAnimationController({
    mouseTarget: new Vector3(0, 1.67, 1),
    render: vi.fn(),
  } as unknown as VrmSceneManager);
  controllers.push(controller);
  controller.setVrm(m.vrm);
  controller.setMotionStyle('legacy');
  controller.updateFrame(0.25);
  expect(chest.scale.toArray()).not.toEqual([1.1, 0.9, 1.05]);
  controller.setMotionStyle('companion');
  controller.updateFrame(0.1);
  expect(chest.scale.toArray()).toEqual([1.1, 0.9, 1.05]);
});

it('connects one-shot gesture previews to the actual rendered rig and returns to rest', () => {
  const m = setup();
  m.controller.updateFrame(0.1);
  const rest = m.position('rightHand');
  m.controller.previewGesture('greet');
  m.controller.updateFrame(1.6);
  expect(m.position('rightHand').y).toBeGreaterThan(rest.y + 0.4);
  m.controller.updateFrame(1.7);
  expect(m.position('rightHand').y).toBeLessThan(rest.y + 0.02);
});

it('hands off imported animation on frame time without a discontinuity or delayed timer', async () => {
  vi.useFakeTimers();
  try {
    const authored = animation();
    authored.humanoidTracks.translation.set(
      'hips',
      new VectorKeyframeTrack('hips.position', [0, 2], [0.15, 1, 0.04, 0.15, 1, 0.04]),
    );
    vi.spyOn(VrmaLoader.prototype, 'loadFile').mockResolvedValue(authored);
    const m = setup();
    await m.controller.loadVrmaAnimationFile(new File(['fixture'], 'manual.vrma'));
    for (let i = 0; i < 60; i++) m.controller.updateFrame(1 / 60);
    const displayed = m.position('rightHand');
    await m.controller.setPose('arms_crossed', 0.5);
    expect(m.controller.isVrmaActive).toBe(false);
    expect(m.position('rightHand').distanceTo(displayed)).toBeLessThan(1e-6);
    for (let i = 0; i < 180; i++) {
      if (i === 12) await m.controller.setPose('thinking', 0.5);
      const before = m.position('rightHand');
      m.controller.updateFrame(1 / 60);
      vi.advanceTimersByTime(1000 / 60);
      expect(m.position('rightHand').distanceTo(before), `handoff frame ${i}`).toBeLessThan(i === 0 ? 0.04 : 0.08);
    }
    expect(m.controller.isVrmaActive).toBe(false);
    expect(m.position('rightHand').y).toBeGreaterThan(1.3);
  } finally {
    vi.useRealTimers();
  }
});

it('preserves frozen imported joints while allowing eye tracking and complete blinks', async () => {
  vi.spyOn(VrmaLoader.prototype, 'loadFile').mockResolvedValue(animation());
  const m = setup();
  await m.controller.loadVrmaPoseFile(new File(['fixture'], 'pose.vrma'));
  let blink = 0;
  const expected = new Quaternion().setFromAxisAngle(new Vector3(1, 0, 0), 0.42);
  for (let i = 0; i < 600; i++) {
    m.controller.updateFrame(1 / 60);
    if (i > 60) expect(m.vrm.humanoid.getNormalizedBoneNode('spine')!.quaternion.angleTo(expected)).toBeLessThan(1e-6);
    blink = Math.max(blink, m.expressions.get('blink') ?? 0);
  }
  expect(blink).toBeGreaterThan(0.98);
  expect(m.vrm.lookAt?.target).not.toBeNull();
});
