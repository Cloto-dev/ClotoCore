import { type VRM, VRMLoaderPlugin, VRMUtils } from '@pixiv/three-vrm';
import * as THREE from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { readGlbDocument } from '../../mate/assetFiles';

/**
 * Loads a VRM model from the kernel API and adds it to a three.js scene.
 * Handles both VRM 0.x and 1.0 via @pixiv/three-vrm v3's unified abstraction.
 */
export class VrmModelLoader {
  private vrm: VRM | null = null;
  private scene: THREE.Scene;

  constructor(scene: THREE.Scene) {
    this.scene = scene;
  }

  /**
   * Load VRM from API endpoint.
   * @param url - Full URL to GET /api/agents/:id/vrm
   * @returns The loaded VRM instance
   */
  async load(url: string): Promise<VRM> {
    // Fetch as ArrayBuffer
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`Failed to fetch VRM: ${response.statusText}`);
    }
    return this.loadBuffer(await response.arrayBuffer());
  }

  async loadFile(file: File): Promise<VRM> {
    return this.loadBuffer(await file.arrayBuffer());
  }

  private async loadBuffer(buffer: ArrayBuffer): Promise<VRM> {
    readGlbDocument(buffer, 'vrm');

    // Setup GLTFLoader with VRM plugin
    const loader = new GLTFLoader();
    loader.register((parser) => new VRMLoaderPlugin(parser));

    // Parse from buffer
    const gltf = await new Promise<import('three/examples/jsm/loaders/GLTFLoader.js').GLTF>((resolve, reject) => {
      loader.parse(buffer, '', resolve, reject);
    });

    const vrm = gltf.userData.vrm as VRM | undefined;
    if (!vrm) {
      throw new Error('Failed to extract VRM from GLTF data');
    }

    // VRM 0 faces +Z; the SDK turns only that format to match VRM 1.
    VRMUtils.rotateVRM0(vrm);

    // Remove previous model if any
    this.disposeVrm();

    this.vrm = vrm;
    this.scene.add(vrm.scene);

    return vrm;
  }

  getVrm(): VRM | null {
    return this.vrm;
  }

  private disposeVrm() {
    if (!this.vrm) return;

    this.scene.remove(this.vrm.scene);

    // Dispose all geometry, material, and textures
    this.vrm.scene.traverse((obj) => {
      if (obj instanceof THREE.Mesh) {
        obj.geometry?.dispose();
        const materials = Array.isArray(obj.material) ? obj.material : [obj.material];
        for (const mat of materials) {
          if (mat && typeof mat.dispose === 'function') {
            // Dispose textures
            for (const key of Object.keys(mat)) {
              const val = (mat as Record<string, unknown>)[key];
              if (val instanceof THREE.Texture) {
                val.dispose();
              }
            }
            mat.dispose();
          }
        }
      }
    });

    this.vrm = null;
  }

  dispose() {
    this.disposeVrm();
  }
}
