import { VRMAnimation } from '@pixiv/three-vrm-animation';
import { QuaternionKeyframeTrack, VectorKeyframeTrack } from 'three';
import { describe, expect, it, vi } from 'vitest';
import { motionRig } from './motionRig.test-utils';
import { VrmaLoader } from './VrmaLoader';

function fixture() {
  const { vrm } = motionRig();
  const loader = new VrmaLoader();
  loader.setVrm(vrm);
  const animation = new VRMAnimation();
  animation.duration = 2;
  animation.restHipsPosition.set(0, 1, 0);
  animation.humanoidTracks.translation.set(
    'hips',
    new VectorKeyframeTrack('hips.position', [0, 2], [0, 1, 0, 2, 1, 0]),
  );
  animation.humanoidTracks.rotation.set(
    'hips',
    new QuaternionKeyframeTrack('hips.quaternion', [0, 2], [0, 0, 0, 1, 0, 0, 0, 1]),
  );
  loader.playAnimation(animation, 0);
  return { loader, animation, hips: vrm.humanoid.getNormalizedBoneNode('hips')! };
}

describe('motion playback', () => {
  it('pauses the real mixer, changes speed and seeks without losing the selected position', () => {
    const { loader, hips } = fixture();
    loader.setSpeed(2);
    loader.update(0.25);
    expect(hips.position.x).toBeCloseTo(0.5);
    loader.setPaused(true);
    loader.update(1);
    expect(loader.playback.time).toBeCloseTo(0.5);
    loader.seek(1.25);
    loader.update(0.5);
    expect(hips.position.x).toBeCloseTo(1.25);
    loader.setPaused(false);
    loader.update(0.1);
    expect(hips.position.x).toBeCloseTo(1.45);
    loader.dispose();
  });
  it('clamps an end seek on the last sample and supports a backward frame step', () => {
    const { loader, hips } = fixture();
    loader.setPaused(true);
    loader.seek(10);
    expect(hips.position.x).toBeCloseTo(2);
    loader.seek(loader.playback.time - 1 / 30);
    expect(hips.position.x).toBeCloseTo(2 - 1 / 30);
    loader.seek(-1);
    expect(hips.position.x).toBe(0);
    loader.dispose();
  });
  it('restarts an already-paused clip from zero at normal speed', () => {
    const { loader, animation } = fixture();
    loader.seek(1);
    loader.setPaused(true);
    loader.setSpeed(0.5);
    loader.playAnimation(animation, 0.5);
    loader.update(0.2);
    expect(loader.playback.paused).toBe(false);
    expect(loader.playback.speed).toBe(1);
    expect(loader.playback.time).toBeCloseTo(0.2);
    loader.dispose();
  });
  it('cancels a deferred stop when immediately stopping, then starting a new clip', () => {
    vi.useFakeTimers();
    try {
      const { loader, animation } = fixture();
      loader.stop(0.5);
      loader.stop(0);
      loader.playAnimation(animation, 0);
      vi.advanceTimersByTime(600);
      expect(loader.playback.active).toBe(true);
      loader.stop(0.5);
      loader.dispose();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
});
