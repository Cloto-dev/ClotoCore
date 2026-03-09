import { VRM } from '@pixiv/three-vrm';

/**
 * Procedural breathing animation.
 * Applies sine-wave rotation to Spine/Chest bones and optional Chest scale modulation.
 */
export class ProceduralBreathing {
  private elapsed = 0;
  // Base period 3.5s, slight randomness via rate param
  private basePeriod = 3.5;
  // Rotation amplitude in radians (~0.7 degrees)
  private baseAmplitude = 0.012;

  update(vrm: VRM, deltaTime: number, breathingRate: number) {
    const humanoid = vrm.humanoid;
    if (!humanoid) return;

    this.elapsed += deltaTime;

    const period = this.basePeriod / Math.max(breathingRate, 0.1);
    const phase = (this.elapsed / period) * Math.PI * 2;
    const breathValue = Math.sin(phase);

    // Spine rotation (primary breathing motion)
    const spine = humanoid.getNormalizedBoneNode('spine');
    if (spine) {
      spine.rotation.x += breathValue * this.baseAmplitude * breathingRate;
    }

    // Chest rotation (secondary, slightly phase-shifted)
    const chest = humanoid.getNormalizedBoneNode('chest');
    if (chest) {
      chest.rotation.x += breathValue * this.baseAmplitude * 0.6 * breathingRate;
    }

    // Subtle chest expansion on inhale (scale uses raw bone — not transferred by normalization)
    const chestRaw = humanoid.getRawBoneNode('chest');
    if (chestRaw) {
      const scaleOffset = 1 + Math.max(0, breathValue) * 0.005 * breathingRate;
      chestRaw.scale.set(scaleOffset, scaleOffset, scaleOffset);
    }
  }

  reset(vrm: VRM) {
    const humanoid = vrm.humanoid;
    if (!humanoid) return;
    const chestRaw = humanoid.getRawBoneNode('chest');
    if (chestRaw) {
      chestRaw.scale.set(1, 1, 1);
    }
  }
}
