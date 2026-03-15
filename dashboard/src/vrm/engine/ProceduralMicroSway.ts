import type { VRM } from '@pixiv/three-vrm';
import { createNoise2D } from 'simplex-noise';

/**
 * Procedural micro-sway animation using simplex noise.
 * Applies subtle head and upper-spine oscillation for lifelike idle motion.
 */
export class ProceduralMicroSway {
  private noiseX = createNoise2D();
  private noiseZ = createNoise2D();
  private elapsed = 0;

  // Base amplitude in radians (~0.5 degrees)
  private baseAmplitude = 0.009;
  // Base frequency ~0.3Hz
  private baseFrequency = 0.3;

  update(vrm: VRM, deltaTime: number, swayAmplitude: number) {
    const humanoid = vrm.humanoid;
    if (!humanoid) return;

    this.elapsed += deltaTime;
    const t = this.elapsed * this.baseFrequency;
    const amplitude = this.baseAmplitude * swayAmplitude;

    // Head sway (primary)
    const head = humanoid.getNormalizedBoneNode('head');
    if (head) {
      head.rotation.x += this.noiseX(t, 0) * amplitude;
      head.rotation.z += this.noiseZ(t, 0) * amplitude * 0.7;
    }

    // Upper spine sway (secondary, slower frequency)
    const upperChest = humanoid.getNormalizedBoneNode('upperChest');
    if (upperChest) {
      upperChest.rotation.x += this.noiseX(t * 0.6, 10) * amplitude * 0.4;
      upperChest.rotation.z += this.noiseZ(t * 0.6, 10) * amplitude * 0.3;
    }
  }
}
