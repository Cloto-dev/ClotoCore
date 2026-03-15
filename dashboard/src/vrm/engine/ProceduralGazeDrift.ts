import type { VRM } from '@pixiv/three-vrm';
import * as THREE from 'three';

/**
 * Procedural gaze drift with micro-saccades.
 * Combines mouse tracking target with small random eye movements.
 */
export class ProceduralGazeDrift {
  private currentTarget = new THREE.Vector3(0, 1.3, 1.0);
  private driftTarget = new THREE.Vector3(0, 0, 0);
  /** Object3D wrapper for VRM lookAt target (requires Object3D, not Vector3) */
  private targetObject = new THREE.Object3D();
  private nextSaccadeIn: number;
  private saccadeAmplitude = 0.03; // radians (~1.7 degrees)

  constructor() {
    this.nextSaccadeIn = this.randomSaccadeInterval();
    this.regenerateDrift();
  }

  private randomSaccadeInterval(): number {
    return 0.5 + Math.random() * 1.5; // 0.5-2s
  }

  private regenerateDrift() {
    // Small random offset in degrees, converted to approximate world-space offset
    const dx = (Math.random() - 0.5) * this.saccadeAmplitude * 2;
    const dy = (Math.random() - 0.5) * this.saccadeAmplitude;
    this.driftTarget.set(dx, dy, 0);
  }

  update(vrm: VRM, deltaTime: number, mouseTarget: THREE.Vector3) {
    if (!vrm.lookAt) return;

    // Micro-saccade timer
    this.nextSaccadeIn -= deltaTime;
    if (this.nextSaccadeIn <= 0) {
      this.regenerateDrift();
      this.nextSaccadeIn = this.randomSaccadeInterval();
    }

    // Combine mouse target + drift offset
    const target = new THREE.Vector3(
      mouseTarget.x + this.driftTarget.x,
      mouseTarget.y + this.driftTarget.y,
      mouseTarget.z + this.driftTarget.z,
    );

    // Smooth interpolation toward target
    this.currentTarget.lerp(target, 1 - Math.exp(-8 * deltaTime));

    // Apply via VRM lookAt (target must be Object3D with position)
    this.targetObject.position.copy(this.currentTarget);
    vrm.lookAt.target = this.targetObject;
  }
}
