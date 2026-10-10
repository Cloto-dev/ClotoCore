import * as THREE from 'three';
import { expect, it } from 'vitest';
import { VrmSceneManager } from './VrmSceneManager';

it('frames the body with finite coordinates when the canvas has not acquired its size', () => {
  // Exercise camera geometry without creating a platform-specific WebGL renderer.
  const manager = Object.create(VrmSceneManager.prototype) as VrmSceneManager;
  const camera = new THREE.PerspectiveCamera(30, Number.NaN, 0.1, 20);
  Object.assign(manager, {
    camera,
    cameraOffset: new THREE.Vector3(),
    lookAtTarget: new THREE.Vector3(),
    mouseTarget: new THREE.Vector3(),
  });
  const model = new THREE.Mesh(new THREE.BoxGeometry(1, 2, 0.2));
  model.position.y = 1;
  manager.frameBody(model);
  expect(camera.position.toArray().every(Number.isFinite)).toBe(true);
  // Once ResizeObserver supplies the real aspect, both extremities fit in view.
  camera.aspect = 1;
  camera.updateProjectionMatrix();
  camera.updateMatrixWorld();
  expect(Math.abs(new THREE.Vector3(0, 0, 0).project(camera).y)).toBeLessThan(1);
  expect(Math.abs(new THREE.Vector3(0, 2, 0).project(camera).y)).toBeLessThan(1);
  model.geometry.dispose();
});

it('makes room for raised fingertips and returns to the same standing camera', () => {
  const manager = Object.create(VrmSceneManager.prototype) as VrmSceneManager;
  const camera = new THREE.PerspectiveCamera(30, 1, 0.1, 20);
  Object.assign(manager, {
    camera,
    cameraOffset: new THREE.Vector3(),
    lookAtTarget: new THREE.Vector3(),
    mouseTarget: new THREE.Vector3(),
  });
  const model = new THREE.Mesh(new THREE.BoxGeometry(1, 2, 0.2));
  model.position.y = 1;
  manager.frameBody(model);
  const standing = camera.position.clone();
  for (let i = 0; i < 180; i++) manager.updateMotionFraming(2.7, 1 / 60);
  camera.updateMatrixWorld();
  expect(new THREE.Vector3(0, 2.7, 0).project(camera).y).toBeLessThan(1);
  expect(new THREE.Vector3(0, 0, 0).project(camera).y).toBeGreaterThan(-1);
  for (let i = 0; i < 300; i++) manager.updateMotionFraming(1, 1 / 60);
  expect(camera.position.distanceTo(standing)).toBeLessThan(1e-8);
  model.geometry.dispose();
});
