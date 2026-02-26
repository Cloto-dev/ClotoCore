import { useEffect, useRef, useState, useMemo } from 'react';
import { FaceLandmarker, FaceLandmarkerResult, FilesetResolver } from '@mediapipe/tasks-vision';
import type { GazeData, DelegateMode, TrackerStatus } from '../types';

type VisionFileset = Awaited<ReturnType<typeof FilesetResolver.forVisionTasks>>;

// --- URLs ---
const MEDIAPIPE_WASM_URL = 'https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@0.10.14/wasm';
const FACE_MODEL_URL = 'https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task';

// --- Detection constants ---
const FACE_DETECTED_CONFIDENCE = 0.85;
const GPU_DETECT_INTERVAL = 33;   // ~30 FPS
const CPU_DETECT_INTERVAL = 66;   // ~15 FPS
const MAX_GPU_ERRORS = 6;
const MAX_CPU_ERRORS = 5;
const CPU_CANVAS_WIDTH = 640;
const CPU_CANVAS_HEIGHT = 480;
const INIT_DELAY_GPU = 1500;
const INIT_DELAY_CPU = 2000;
const ERROR_LOGGING_THRESHOLD = 3;
const ZERO_DIVISION_EPSILON = 0.0001;

// --- One Euro Filter parameters ---
// mincutoff: minimum cutoff frequency (Hz). Lower = less jitter at rest, more lag.
// beta: speed coefficient. Higher = less lag during fast eye movement.
// dcutoff: derivative cutoff frequency (Hz). Rarely needs tuning.
const ONE_EURO_MINCUTOFF = 1.0;
const ONE_EURO_BETA = 0.5;
const ONE_EURO_DCUTOFF = 1.0;
const ONE_EURO_INITIAL_FREQ = 15; // Hz, auto-adjusts based on actual frame timing

// --- Gaze mapping parameters ---
const GAIN_X = 1.5;
const GAIN_Y = 1.5;
const DEAD_ZONE_PX = 50; // Pixels — cursor won't move if gaze shifts less than this
const SACCADE_VELOCITY_THRESHOLD = 0.15; // Normalized units/sec — above this = saccade

// --- Iris landmark indices (MediaPipe 478-point model) ---
const RIGHT_IRIS_CENTER = 468;
const LEFT_IRIS_CENTER = 473;
const RIGHT_EYE_OUTER = 33;
const RIGHT_EYE_INNER = 133;
const LEFT_EYE_OUTER = 263;
const LEFT_EYE_INNER = 362;
const RIGHT_EYE_TOP = 159;
const RIGHT_EYE_BOTTOM = 145;
const LEFT_EYE_TOP = 386;
const LEFT_EYE_BOTTOM = 374;
const MIN_LANDMARKS_FOR_IRIS = 474;

// --- GPU blocklist / warnlist ---
const GPU_BLOCKLIST: RegExp[] = [
  /swiftshader/i,
  /llvmpipe/i,
  /software/i,
  /microsoft basic/i,
];
const GPU_WARNLIST: RegExp[] = [
  /radeon.*r9\s*2/i,
  /radeon.*r7\s*2/i,
  /radeon.*hd\s*7/i,
  /radeon.*r9\s*3[78]/i,
  /mali-t/i,
  /intel.*hd.*4[0-9]{3}/i,
];

// ============================================================
// One Euro Filter — adaptive low-pass filter for noisy signals
// Reference: Casiez et al., CHI 2012
// https://gery.casiez.net/1euro/
// ============================================================

class LowPassFilter {
  private s = 0;
  private initialized = false;
  private raw = 0;

  hasLastRaw(): boolean { return this.initialized; }
  lastRaw(): number { return this.raw; }

  filter(value: number, alpha: number): number {
    this.raw = value;
    if (this.initialized) {
      this.s = alpha * value + (1 - alpha) * this.s;
    } else {
      this.s = value;
      this.initialized = true;
    }
    return this.s;
  }
}

class OneEuroFilter {
  private freq: number;
  private mincutoff: number;
  private beta: number;
  private dcutoff: number;
  private xFilt = new LowPassFilter();
  private dxFilt = new LowPassFilter();
  private lastTime = -1;

  constructor(freq: number, mincutoff: number, beta: number, dcutoff: number) {
    this.freq = freq;
    this.mincutoff = mincutoff;
    this.beta = beta;
    this.dcutoff = dcutoff;
  }

  private alpha(cutoff: number): number {
    const te = 1.0 / this.freq;
    const tau = 1.0 / (2 * Math.PI * cutoff);
    return 1.0 / (1.0 + tau / te);
  }

  filter(value: number, timestamp: number): number {
    // Auto-adjust frequency from timestamps
    if (this.lastTime >= 0) {
      const dt = timestamp - this.lastTime;
      if (dt > 0) this.freq = 1.0 / dt;
    }
    this.lastTime = timestamp;

    // Estimate derivative
    const dvalue = this.xFilt.hasLastRaw()
      ? (value - this.xFilt.lastRaw()) * this.freq
      : 0;
    const edvalue = this.dxFilt.filter(dvalue, this.alpha(this.dcutoff));

    // Adaptive cutoff: low when still (jitter removal), high when moving (low lag)
    const cutoff = this.mincutoff + this.beta * Math.abs(edvalue);
    return this.xFilt.filter(value, this.alpha(cutoff));
  }
}

// ============================================================
// Pure helper functions
// ============================================================

/**
 * Detects GPU capability for MediaPipe GPU delegate.
 *
 * Uses a SEPARATE canvas for probing to avoid resource conflicts with MediaPipe.
 * After probing, the canvas is discarded (GC handles context cleanup).
 * We do NOT call WEBGL_lose_context because:
 * - It is asynchronous and can race with MediaPipe's WebGL context creation
 * - Discarding the canvas element achieves the same cleanup without race conditions
 */
async function detectGPUCapability(): Promise<{ canUseGPU: boolean; renderer: string; reason: string }> {
  const result = { canUseGPU: false, renderer: 'unknown', reason: '' };

  // Use a separate canvas that will be GC'd after this function
  const probeCanvas = document.createElement('canvas');
  probeCanvas.width = 1;
  probeCanvas.height = 1;
  let gl: WebGL2RenderingContext | null = null;
  let performanceCaveat = false;

  // Try with failIfMajorPerformanceCaveat to detect software/slow GPUs.
  // If it fails, we still allow GPU — the smoke test and runtime fallback
  // provide additional safety layers. This avoids incorrectly blocking
  // capable GPUs like RDNA3 that report false performance caveats on ANGLE/D3D11.
  try {
    gl = probeCanvas.getContext('webgl2', {
      failIfMajorPerformanceCaveat: true,
      powerPreference: 'high-performance',
    });
  } catch {
    // Some browsers throw instead of returning null
  }

  if (!gl) {
    performanceCaveat = true;
    // Retry on a fresh canvas without the caveat flag
    const fallbackCanvas = document.createElement('canvas');
    fallbackCanvas.width = 1;
    fallbackCanvas.height = 1;
    try { gl = fallbackCanvas.getContext('webgl2'); } catch { /* ignore */ }
    if (!gl) {
      result.reason = 'WebGL 2.0 not available';
      return result;
    }
  }

  // Query renderer name
  const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
  if (debugInfo) {
    result.renderer = gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL) || 'unknown';
  } else {
    result.renderer = gl.getParameter(gl.RENDERER) || 'unknown';
  }

  // Do NOT call loseContext() — let GC handle cleanup to avoid async race with MediaPipe.
  // The probeCanvas goes out of scope and gets collected.

  const isApproximated = /or similar/i.test(result.renderer);

  for (const pattern of GPU_BLOCKLIST) {
    if (pattern.test(result.renderer)) {
      result.reason = `Software renderer: ${result.renderer}`;
      return result;
    }
  }

  if (!isApproximated) {
    for (const pattern of GPU_WARNLIST) {
      if (pattern.test(result.renderer)) {
        result.reason = `Older GPU (GCN/legacy): ${result.renderer}`;
        return result;
      }
    }
  }

  result.canUseGPU = true;
  const caveatNote = performanceCaveat ? ' (performance caveat reported, proceeding anyway)' : '';
  result.reason = isApproximated
    ? `GPU allowed (Firefox privacy-approximated name): ${result.renderer}${caveatNote}`
    : `Compatible GPU: ${result.renderer}${caveatNote}`;
  return result;
}

function safeRatio(numerator: number, denominator: number, fallback = 0.5): number {
  if (Math.abs(denominator) < ZERO_DIVISION_EPSILON) return fallback;
  const r = numerator / denominator;
  return Number.isFinite(r) ? r : fallback;
}

/**
 * Extracts raw gaze ratios from MediaPipe face landmarks.
 * Returns normalized [0,1] ratios for X and Y axes.
 * No smoothing or screen mapping — pure geometric extraction.
 */
function extractGazeRatios(
  lm: { x: number; y: number }[],
): { rawX: number; rawY: number } | null {
  if (lm.length < MIN_LANDMARKS_FOR_IRIS) return null;

  const rightGazeX = safeRatio(
    lm[RIGHT_IRIS_CENTER].x - lm[RIGHT_EYE_OUTER].x,
    lm[RIGHT_EYE_INNER].x - lm[RIGHT_EYE_OUTER].x,
  );
  const leftGazeX = safeRatio(
    lm[LEFT_IRIS_CENTER].x - lm[LEFT_EYE_OUTER].x,
    lm[LEFT_EYE_INNER].x - lm[LEFT_EYE_OUTER].x,
  );
  const rightGazeY = safeRatio(
    lm[RIGHT_IRIS_CENTER].y - lm[RIGHT_EYE_TOP].y,
    lm[RIGHT_EYE_BOTTOM].y - lm[RIGHT_EYE_TOP].y,
  );
  const leftGazeY = safeRatio(
    lm[LEFT_IRIS_CENTER].y - lm[LEFT_EYE_TOP].y,
    lm[LEFT_EYE_BOTTOM].y - lm[LEFT_EYE_TOP].y,
  );

  // Left eye X ratio runs opposite to right eye due to mirrored geometry
  const rawX = (rightGazeX + (1 - leftGazeX)) / 2;
  const rawY = (rightGazeY + leftGazeY) / 2;

  return { rawX, rawY };
}

/**
 * Maps filtered gaze ratios to screen coordinates with dead zone.
 *
 * Pipeline:
 * 1. Apply GAIN to expand the usable ratio range to full screen
 * 2. Map to screen pixel coordinates
 * 3. Apply dead zone: if new position is within DEAD_ZONE_PX of current,
 *    keep the current position to eliminate residual jitter
 */
function mapGazeToScreen(
  smoothX: number,
  smoothY: number,
  lastPos: { x: number; y: number },
): GazeData {
  const expandedX = 0.5 + (smoothX - 0.5) * GAIN_X;
  const expandedY = 0.5 + (smoothY - 0.5) * GAIN_Y;

  const screenW = window.innerWidth;
  const screenH = window.innerHeight;
  // X: mirror because raw webcam image is not flipped — user's right appears on image left
  // Y: direct mapping — looking down increases both rawY and screen Y
  const candidateX = Math.max(0, Math.min(screenW - 1, Math.round((1 - expandedX) * screenW)));
  const candidateY = Math.max(0, Math.min(screenH - 1, Math.round(expandedY * screenH)));

  // Dead zone: suppress micro-movements within threshold
  const dx = candidateX - lastPos.x;
  const dy = candidateY - lastPos.y;
  const dist = Math.sqrt(dx * dx + dy * dy);
  const insideDeadZone = dist < DEAD_ZONE_PX && lastPos.x >= 0;

  return {
    x: insideDeadZone ? lastPos.x : candidateX,
    y: insideDeadZone ? lastPos.y : candidateY,
    confidence: FACE_DETECTED_CONFIDENCE,
    fixated: insideDeadZone,
  };
}

function dispatchGaze(data: GazeData) {
  window.dispatchEvent(new CustomEvent('cloto-gaze', { detail: data }));
}

function createLandmarker(
  fileset: VisionFileset,
  delegate: 'GPU' | 'CPU',
  mode: 'VIDEO' | 'IMAGE',
): Promise<FaceLandmarker> {
  return FaceLandmarker.createFromOptions(fileset, {
    baseOptions: { modelAssetPath: FACE_MODEL_URL, delegate },
    runningMode: mode,
    numFaces: 1,
    minFaceDetectionConfidence: 0.5,
    minFacePresenceConfidence: 0.5,
    minTrackingConfidence: 0.5,
    outputFaceBlendshapes: false,
    outputFacialTransformationMatrixes: false,
  });
}

function noFaceGaze(): GazeData {
  return {
    x: Math.round(window.innerWidth / 2),
    y: Math.round(window.innerHeight / 2),
    confidence: 0,
    fixated: false,
  };
}

// ============================================================
// Detection loop config
// ============================================================

const EMPTY_RESULT: FaceLandmarkerResult = {
  faceLandmarks: [],
  faceBlendshapes: [],
  facialTransformationMatrixes: [],
};

interface DetectionLoopRefs {
  landmarkerRef: React.MutableRefObject<FaceLandmarker | null>;
  timerRef: React.MutableRefObject<number>;
  errorCountRef: React.MutableRefObject<number>;
  cancelledRef: React.MutableRefObject<boolean>;
  lastTimestampRef: React.MutableRefObject<number>;
  lastVideoTimeRef: React.MutableRefObject<number>;
  filterXRef: React.MutableRefObject<OneEuroFilter>;
  filterYRef: React.MutableRefObject<OneEuroFilter>;
  lastScreenPosRef: React.MutableRefObject<{ x: number; y: number }>;
}

interface DetectionConfig {
  interval: number;
  maxErrors: number;
  initDelay: number;
  detect: (landmarker: FaceLandmarker) => FaceLandmarkerResult;
  onMaxErrors: () => void;
  onResult: (gazeData: GazeData) => void;
  onNoFace: () => void;
}

function startDetectionLoop(refs: DetectionLoopRefs, config: DetectionConfig) {
  const loop = () => {
    if (refs.cancelledRef.current) return;

    if (refs.errorCountRef.current >= config.maxErrors) {
      config.onMaxErrors();
      return;
    }

    if (refs.landmarkerRef.current) {
      try {
        const t0 = performance.now();
        const result = config.detect(refs.landmarkerRef.current);
        const elapsed = performance.now() - t0;
        const nextInterval = Math.max(config.interval, elapsed * 1.5);

        if (result.faceLandmarks && result.faceLandmarks.length > 0) {
          const raw = extractGazeRatios(result.faceLandmarks[0]);
          if (raw) {
            refs.errorCountRef.current = 0;

            // One Euro Filter: adaptive smoothing (timestamp in seconds)
            const now = performance.now() / 1000;
            const smoothX = refs.filterXRef.current.filter(raw.rawX, now);
            const smoothY = refs.filterYRef.current.filter(raw.rawY, now);

            // Map to screen with dead zone
            const gazeData = mapGazeToScreen(
              smoothX,
              smoothY,
              refs.lastScreenPosRef.current,
            );

            // Update anchor only when cursor actually moved (outside dead zone)
            if (!gazeData.fixated) {
              refs.lastScreenPosRef.current = { x: gazeData.x, y: gazeData.y };
            }

            config.onResult(gazeData);
            dispatchGaze(gazeData);
          }
        } else {
          config.onNoFace();
        }

        refs.timerRef.current = window.setTimeout(loop, nextInterval);
        return;
      } catch (err) {
        refs.errorCountRef.current++;
        if (refs.errorCountRef.current <= ERROR_LOGGING_THRESHOLD) {
          console.warn(`Detection error (${refs.errorCountRef.current}/${config.maxErrors}):`, err);
        }
      }
    }

    refs.timerRef.current = window.setTimeout(loop, config.interval);
  };

  refs.timerRef.current = window.setTimeout(loop, config.initDelay);
}

// ============================================================
// Hook
// ============================================================

export function useGazeDetection() {
  const videoRef = useRef<HTMLVideoElement>(null);
  const landmarkerRef = useRef<FaceLandmarker | null>(null);
  const timerRef = useRef<number>(0);
  const errorCountRef = useRef<number>(0);
  const lastTimestampRef = useRef<number>(0);
  const lastVideoTimeRef = useRef<number>(-1);
  const initRef = useRef(false);
  const offCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const offCtxRef = useRef<CanvasRenderingContext2D | null>(null);
  const delegateModeRef = useRef<DelegateMode>('cpu');
  const streamRef = useRef<MediaStream | null>(null);
  const cancelledRef = useRef(false);
  const filesetRef = useRef<VisionFileset | null>(null);

  // One Euro Filter instances (one per axis)
  const filterXRef = useRef(new OneEuroFilter(ONE_EURO_INITIAL_FREQ, ONE_EURO_MINCUTOFF, ONE_EURO_BETA, ONE_EURO_DCUTOFF));
  const filterYRef = useRef(new OneEuroFilter(ONE_EURO_INITIAL_FREQ, ONE_EURO_MINCUTOFF, ONE_EURO_BETA, ONE_EURO_DCUTOFF));
  // Dead zone anchor: last committed screen position
  const lastScreenPosRef = useRef<{ x: number; y: number }>({ x: -1, y: -1 });

  const [gaze, setGaze] = useState<GazeData | null>(null);
  const [status, setStatus] = useState<TrackerStatus>('loading');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [delegateLabel, setDelegateLabel] = useState<string>('');

  const loopRefs: DetectionLoopRefs = useMemo(
    () => ({
      landmarkerRef,
      timerRef,
      errorCountRef,
      cancelledRef,
      lastTimestampRef,
      lastVideoTimeRef,
      filterXRef,
      filterYRef,
      lastScreenPosRef,
    }),
    [],
  );

  const buildCpuDetectFn = (video: HTMLVideoElement) => {
    const offCanvas = document.createElement('canvas');
    offCanvas.width = CPU_CANVAS_WIDTH;
    offCanvas.height = CPU_CANVAS_HEIGHT;
    offCanvasRef.current = offCanvas;
    const offCtx = offCanvas.getContext('2d', { willReadFrequently: true });
    offCtxRef.current = offCtx;

    return (landmarker: FaceLandmarker): FaceLandmarkerResult => {
      if (
        video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA ||
        video.videoWidth === 0 || video.videoHeight === 0 ||
        !offCtx
      ) {
        return EMPTY_RESULT;
      }
      offCtx.drawImage(video, 0, 0, CPU_CANVAS_WIDTH, CPU_CANVAS_HEIGHT);
      return landmarker.detect(offCanvas);
    };
  };

  const buildGpuDetectFn = (video: HTMLVideoElement) => {
    return (landmarker: FaceLandmarker): FaceLandmarkerResult => {
      if (
        video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA ||
        video.videoWidth === 0 || video.videoHeight === 0 ||
        video.currentTime === lastVideoTimeRef.current
      ) {
        return EMPTY_RESULT;
      }
      lastVideoTimeRef.current = video.currentTime;
      const now = performance.now();
      const timestamp = now > lastTimestampRef.current ? now : lastTimestampRef.current + 1;
      lastTimestampRef.current = timestamp;
      return landmarker.detectForVideo(video, timestamp);
    };
  };

  const startCpuDetection = (video: HTMLVideoElement, onMaxErrors?: () => void) => {
    startDetectionLoop(loopRefs, {
      interval: CPU_DETECT_INTERVAL,
      maxErrors: MAX_CPU_ERRORS,
      initDelay: INIT_DELAY_CPU,
      detect: buildCpuDetectFn(video),
      onMaxErrors: onMaxErrors ?? (() => {
        console.error(`CPU detection stopped after ${MAX_CPU_ERRORS} errors.`);
        setStatus('stopped');
        setErrorMsg('Detection stopped. Try reloading.');
      }),
      onResult: (gazeData) => setGaze(gazeData),
      onNoFace: () => setGaze(noFaceGaze()),
    });
  };

  const fallbackToCpu = async (video: HTMLVideoElement) => {
    if (cancelledRef.current) return;

    try {
      if (landmarkerRef.current) {
        landmarkerRef.current.close();
        landmarkerRef.current = null;
      }

      console.debug('Creating CPU FaceLandmarker (fallback)...');
      const fileset = filesetRef.current;
      if (!fileset) throw new Error('Fileset not loaded');

      const cpuLandmarker = await createLandmarker(fileset, 'CPU', 'IMAGE');
      if (cancelledRef.current) { cpuLandmarker.close(); return; }

      landmarkerRef.current = cpuLandmarker;
      delegateModeRef.current = 'cpu-fallback';
      errorCountRef.current = 0;
      setDelegateLabel('CPU fallback');
      console.debug('CPU fallback active.');

      startCpuDetection(video);
    } catch (err) {
      console.error('CPU fallback init failed:', err);
      setStatus('error');
      setErrorMsg('Failed to initialize CPU fallback');
    }
  };

  const startGpuDetection = (video: HTMLVideoElement) => {
    startDetectionLoop(loopRefs, {
      interval: GPU_DETECT_INTERVAL,
      maxErrors: MAX_GPU_ERRORS,
      initDelay: INIT_DELAY_GPU,
      detect: buildGpuDetectFn(video),
      onMaxErrors: () => {
        console.warn(`GPU delegate failed at runtime (${MAX_GPU_ERRORS} errors). Falling back to CPU...`);
        fallbackToCpu(video);
      },
      onResult: (gazeData) => setGaze(gazeData),
      onNoFace: () => setGaze(noFaceGaze()),
    });
  };

  useEffect(() => {
    if (initRef.current) return;
    initRef.current = true;
    cancelledRef.current = false;

    const init = async () => {
      try {
        // Step 1: Detect GPU capability
        const gpuInfo = await detectGPUCapability();
        console.debug(`GPU detection: canUseGPU=${gpuInfo.canUseGPU}, renderer="${gpuInfo.renderer}", reason="${gpuInfo.reason}"`);

        // Step 2: Load WASM fileset
        console.debug('Loading MediaPipe WASM...');
        const fileset = await FilesetResolver.forVisionTasks(MEDIAPIPE_WASM_URL);
        if (cancelledRef.current) return;
        filesetRef.current = fileset;

        // Step 3: Try GPU, fallback to CPU
        let landmarker: FaceLandmarker;
        let useGpu = false;

        if (gpuInfo.canUseGPU) {
          try {
            console.debug('Attempting GPU delegate (VIDEO mode)...');
            landmarker = await createLandmarker(fileset, 'GPU', 'VIDEO');
            if (cancelledRef.current) { landmarker.close(); return; }

            // Smoke test: use detectForVideo() (correct API for VIDEO mode).
            // Previous code used detect() which is IMAGE mode API — this caused
            // false failures on GPU delegates that only support VIDEO mode operations.
            const testCanvas = document.createElement('canvas');
            testCanvas.width = 320;
            testCanvas.height = 240;
            const testCtx = testCanvas.getContext('2d');
            if (testCtx) testCtx.fillRect(0, 0, 320, 240);
            landmarker.detectForVideo(testCanvas, performance.now());
            console.debug('GPU smoke test passed (detectForVideo).');
            useGpu = true;
          } catch (err) {
            console.warn('GPU delegate failed during init/smoke test:', err);
            // Retry once — transient WebGL init failures are common on ANGLE/AMD
            try {
              console.debug('Retrying GPU delegate...');
              try { landmarker!.close(); } catch { /* ignore */ }
              await new Promise(r => setTimeout(r, 200));
              landmarker = await createLandmarker(fileset, 'GPU', 'VIDEO');
              if (cancelledRef.current) { landmarker.close(); return; }
              const retryCanvas = document.createElement('canvas');
              retryCanvas.width = 320;
              retryCanvas.height = 240;
              const retryCtx = retryCanvas.getContext('2d');
              if (retryCtx) retryCtx.fillRect(0, 0, 320, 240);
              landmarker.detectForVideo(retryCanvas, performance.now());
              console.debug('GPU smoke test passed on retry.');
              useGpu = true;
            } catch (retryErr) {
              console.warn('GPU delegate failed on retry, falling back to CPU:', retryErr);
              try { landmarker!.close(); } catch { /* ignore */ }
              landmarker = await createLandmarker(fileset, 'CPU', 'IMAGE');
              if (cancelledRef.current) { landmarker.close(); return; }
              delegateModeRef.current = 'cpu-fallback';
              setDelegateLabel('CPU fallback');
            }
          }
        } else {
          console.debug(`GPU blocked: ${gpuInfo.reason}. Using CPU delegate.`);
          landmarker = await createLandmarker(fileset, 'CPU', 'IMAGE');
          if (cancelledRef.current) { landmarker.close(); return; }
        }

        landmarkerRef.current = landmarker;

        if (useGpu) {
          delegateModeRef.current = 'gpu';
          setDelegateLabel('GPU');
          console.debug('FaceLandmarker ready (GPU, VIDEO mode).');
        } else if (delegateModeRef.current !== 'cpu-fallback') {
          delegateModeRef.current = 'cpu';
          setDelegateLabel('CPU');
          console.debug('FaceLandmarker ready (CPU, IMAGE mode).');
        }

        // Step 4: Request camera
        setStatus('requesting');
        console.debug('Requesting camera access...');
        const cameraWidth = useGpu ? 960 : 640;
        const cameraHeight = useGpu ? 720 : 480;
        const stream = await navigator.mediaDevices.getUserMedia({
          video: { width: { ideal: cameraWidth }, height: { ideal: cameraHeight }, facingMode: 'user' },
        });
        if (cancelledRef.current) { stream.getTracks().forEach(t => t.stop()); return; }
        streamRef.current = stream;

        const video = videoRef.current;
        if (!video) {
          stream.getTracks().forEach(t => t.stop());
          return;
        }
        video.srcObject = stream;
        await video.play();
        setStatus('active');
        console.debug(`Camera active (${video.videoWidth}x${video.videoHeight}). Starting ${useGpu ? 'GPU' : 'CPU'} detection loop.`);

        // Step 5: Start detection loop
        if (useGpu) {
          startGpuDetection(video);
        } else {
          startCpuDetection(video);
        }
      } catch (err: unknown) {
        if (cancelledRef.current) return;
        console.error('GazeTracker init error:', err);
        const error = err as { name?: string; message?: string };
        if (error.name === 'NotAllowedError') {
          setStatus('denied');
          setErrorMsg('Camera access denied.');
        } else {
          setStatus('error');
          setErrorMsg(error.message || 'Initialization failed');
        }
      }
    };

    init();

    return () => {
      cancelledRef.current = true;
      if (timerRef.current) clearTimeout(timerRef.current);
      if (streamRef.current) streamRef.current.getTracks().forEach(t => t.stop());
      if (videoRef.current) videoRef.current.srcObject = null;
      if (landmarkerRef.current) {
        landmarkerRef.current.close();
        landmarkerRef.current = null;
      }
      offCanvasRef.current = null;
      offCtxRef.current = null;
      initRef.current = false;
    };
  }, []);

  const fpsLabel = delegateModeRef.current === 'gpu' ? '~30fps' : '~15fps';

  return { videoRef, gaze, status, errorMsg, delegateLabel, fpsLabel };
}
