import * as THREE from 'three';

/**
 * Manages the three.js Scene, Camera, Renderer, and lighting
 * for VRM avatar rendering on a given <canvas> element.
 */
export class VrmSceneManager {
  readonly scene: THREE.Scene;
  readonly camera: THREE.PerspectiveCamera;
  readonly renderer: THREE.WebGLRenderer;

  /** Normalized mouse position in [-1, 1] range (canvas-local) */
  readonly mouseTarget = new THREE.Vector3(0, 0, 1);

  private resizeObserver: ResizeObserver | null = null;
  private canvas: HTMLCanvasElement;
  private _disposed = false;

  // Camera orbit state
  private cameraOffset = new THREE.Vector3(0, 1.35, 1.5);
  private bodyFraming: { height: number; top: number; extra: number } | null = null;
  private lookAtTarget = new THREE.Vector3(0, 1.3, 0);
  private headY = 1.3; // updated by frameHead()
  private isOrbiting = false;
  private orbitStart = { x: 0, y: 0 };

  // Cross-window gaze: true when local mouse is inside this canvas
  private mouseIsLocal = false;
  private gazeChannel: BroadcastChannel | null = null;

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;

    // Scene
    this.scene = new THREE.Scene();

    // Camera — fov 30 for upper-body framing, close distance
    this.camera = new THREE.PerspectiveCamera(
      30,
      Math.max(1, canvas.clientWidth) / Math.max(1, canvas.clientHeight),
      0.1,
      20,
    );
    this.camera.position.set(0, 1.35, 1.5);
    this.camera.lookAt(0, 1.3, 0);

    // Renderer — transparent background for overlay on dashboard
    this.renderer = new THREE.WebGLRenderer({
      canvas,
      alpha: true,
      antialias: true,
    });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.renderer.setSize(canvas.clientWidth, canvas.clientHeight, false);
    this.renderer.setClearColor(0x000000, 0); // fully transparent
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    // Keep the authored SDR colors instead of applying a cinematic contrast curve.
    this.renderer.toneMapping = THREE.NoToneMapping;

    // Lighting
    // MToon uses Lambert's 1 / PI normalization. Split a PI-strength light
    // between soft fill and a key so faces stay readable without bleaching textures.
    const ambient = new THREE.AmbientLight(0xffffff, Math.PI * 0.6);
    this.scene.add(ambient);
    const directional = new THREE.DirectionalLight(0xffffff, Math.PI * 0.4);
    directional.position.set(1, 2, 3);
    this.scene.add(directional);

    // Resize observer
    this.resizeObserver = new ResizeObserver(() => this.handleResize());
    this.resizeObserver.observe(canvas);

    // Mouse tracking + camera controls
    canvas.addEventListener('mousemove', this.handleMouseMove);
    canvas.addEventListener('mouseenter', this.handleMouseEnter);
    canvas.addEventListener('mouseleave', this.handleMouseLeave);
    canvas.addEventListener('wheel', this.handleWheel, { passive: false });
    canvas.addEventListener('mousedown', this.handleMouseDown);
    canvas.addEventListener('mouseup', this.handleMouseUp);
    canvas.addEventListener('contextmenu', (e) => e.preventDefault());

    // Listen for gaze coordinates from other windows via BroadcastChannel
    try {
      this.gazeChannel = new BroadcastChannel('cloto-vrm-gaze');
      this.gazeChannel.onmessage = (e: MessageEvent) => {
        if (this._disposed || this.mouseIsLocal || this.isOrbiting) return;
        const { nx, ny } = e.data;
        if (typeof nx === 'number' && typeof ny === 'number') {
          this.mouseTarget.set(nx * 0.5, this.headY + ny * 0.3, 1.0);
        }
      };
    } catch {
      /* BroadcastChannel not supported, local-only gaze */
    }
  }

  private handleResize() {
    if (this._disposed) return;
    const w = this.canvas.clientWidth;
    const h = this.canvas.clientHeight;
    if (w === 0 || h === 0) return;
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.renderer.setSize(w, h, false);
  }

  private handleMouseMove = (e: MouseEvent) => {
    // Orbit when middle button is held
    if (this.isOrbiting) {
      const dx = (e.clientX - this.orbitStart.x) * 0.005;
      const dy = (e.clientY - this.orbitStart.y) * 0.005;
      this.orbitStart = { x: e.clientX, y: e.clientY };

      // Rotate camera around lookAtTarget
      const offset = this.cameraOffset.clone().sub(this.lookAtTarget);
      const spherical = new THREE.Spherical().setFromVector3(offset);
      spherical.theta -= dx; // horizontal orbit
      spherical.phi -= dy; // vertical orbit
      // Clamp phi to avoid flipping (5°–175°)
      spherical.phi = Math.max(0.09, Math.min(Math.PI - 0.09, spherical.phi));
      offset.setFromSpherical(spherical);
      this.cameraOffset.copy(this.lookAtTarget).add(offset);
      this.applyCameraTransform();
      return;
    }

    // Normal gaze tracking
    const rect = this.canvas.getBoundingClientRect();
    const nx = ((e.clientX - rect.left) / rect.width) * 2 - 1;
    const ny = -((e.clientY - rect.top) / rect.height) * 2 + 1;
    this.mouseTarget.set(nx * 0.5, this.headY + ny * 0.3, 1.0);
  };

  private handleMouseEnter = () => {
    this.mouseIsLocal = true;
  };

  private handleMouseLeave = () => {
    this.mouseIsLocal = false;
    this.mouseTarget.set(0, this.headY, 1.0);
    this.isOrbiting = false;
  };

  private handleWheel = (e: WheelEvent) => {
    e.preventDefault();
    const zoomSpeed = 0.002;
    const delta = e.deltaY * zoomSpeed;
    const dir = new THREE.Vector3().subVectors(this.lookAtTarget, this.cameraOffset).normalize();
    this.cameraOffset.addScaledVector(dir, -delta);
    // Clamp distance to prevent clipping or going too far
    const dist = this.cameraOffset.distanceTo(this.lookAtTarget);
    if (dist < 0.3) {
      this.cameraOffset.copy(this.lookAtTarget).addScaledVector(dir, -0.3);
    } else if (dist > 5) {
      this.cameraOffset.copy(this.lookAtTarget).addScaledVector(dir, -5);
    }
    this.applyCameraTransform();
  };

  private handleMouseDown = (e: MouseEvent) => {
    if (e.button === 1) {
      e.preventDefault();
      this.isOrbiting = true;
      this.orbitStart = { x: e.clientX, y: e.clientY };
    }
  };

  private handleMouseUp = (e: MouseEvent) => {
    if (e.button === 1) {
      this.isOrbiting = false;
    }
  };

  private applyCameraTransform() {
    this.camera.position.copy(this.cameraOffset);
    this.camera.lookAt(this.lookAtTarget);
  }

  /** Reframe camera so the VRM head bone is at vertical center of viewport. */
  frameHead(y: number) {
    this.headY = y;
    this.lookAtTarget.y = y;
    this.cameraOffset.y = y;
    this.mouseTarget.y = y;
    this.applyCameraTransform();
  }

  /** Fit the whole partner inside the presence area, including avatars of different heights. */
  frameBody(model: THREE.Object3D) {
    const box = new THREE.Box3().setFromObject(model);
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3());
    this.bodyFraming = { height: Math.max(0.1, size.y), top: box.max.y, extra: 0 };
    const aspect = Number.isFinite(this.camera.aspect) && this.camera.aspect > 0 ? this.camera.aspect : 1;
    const distance =
      (Math.max(size.y, size.x / Math.max(aspect, 0.1)) /
        (2 * Math.tan(THREE.MathUtils.degToRad(this.camera.fov / 2)))) *
      1.2;
    this.lookAtTarget.copy(center);
    this.cameraOffset.set(center.x, center.y, center.z + Math.max(0.5, distance));
    this.headY = box.max.y * 0.9;
    this.applyCameraTransform();
  }

  /** Keep raised hands visible without changing the user's orbit or standing framing. */
  updateMotionFraming(highestY: number, dt: number) {
    const frame = this.bodyFraming;
    if (!frame || !Number.isFinite(highestY)) return;
    const desired =
      highestY > frame.top - frame.height * 0.04 ? Math.max(0, (highestY - frame.top) / frame.height + 0.08) : 0;
    const next = frame.extra + (desired - frame.extra) * (1 - Math.exp(-8 * Math.max(0, dt)));
    const offset = this.cameraOffset
      .clone()
      .sub(this.lookAtTarget)
      .multiplyScalar((1 + next) / (1 + frame.extra));
    this.lookAtTarget.y += (next - frame.extra) * frame.height * 0.5;
    this.cameraOffset.copy(this.lookAtTarget).add(offset);
    frame.extra = next;
    this.applyCameraTransform();
  }

  render() {
    if (this._disposed) return;
    this.renderer.render(this.scene, this.camera);
  }

  dispose() {
    this._disposed = true;
    this.canvas.removeEventListener('mousemove', this.handleMouseMove);
    this.canvas.removeEventListener('mouseenter', this.handleMouseEnter);
    this.canvas.removeEventListener('mouseleave', this.handleMouseLeave);
    this.canvas.removeEventListener('wheel', this.handleWheel);
    this.canvas.removeEventListener('mousedown', this.handleMouseDown);
    this.canvas.removeEventListener('mouseup', this.handleMouseUp);
    this.gazeChannel?.close();
    this.gazeChannel = null;
    this.resizeObserver?.disconnect();
    this.resizeObserver = null;
    this.renderer.dispose();
    this.scene.clear();
  }
}
