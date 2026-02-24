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
const CPU_DETECT_INTERVAL = 250;  // ~4 FPS
const MAX_GPU_ERRORS = 3;
const MAX_CPU_ERRORS = 5;
const GPU_ALPHA = 0.5;
const CPU_ALPHA = 0.4;
const CPU_CANVAS_WIDTH = 320;
const CPU_CANVAS_HEIGHT = 240;
const INIT_DELAY_GPU = 1500;
const INIT_DELAY_CPU = 2000;
const ERROR_LOGGING_THRESHOLD = 3;  // Log errors only for first N occurrences
const FIXATION_CENTER_DISTANCE_THRESHOLD = 0.08;  // Normalized distance from center (0-1)
const ZERO_DIVISION_EPSILON = 0.0001;  // Eye landmark coordinates are normalized [0,1]

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
// Pure helper functions
// ============================================================

/**
 * Detects GPU capability for MediaPipe GPU delegate.
 *
 * Strategy:
 * 1. Create WebGL2 context with high-performance preference
 * 2. Query GPU renderer name via WEBGL_debug_renderer_info extension
 * 3. Match against blocklist (software renderers) and warnlist (legacy GPUs)
 * 4. Release WebGL context to avoid resource conflicts with MediaPipe
 *
 * Why this is needed:
 * - MediaPipe GPU delegate uses WebGL internally
 * - Older GPUs (GCN 1.0, Mali-T) crash with GPU delegate due to driver bugs
 * - Software renderers (SwiftShader, llvmpipe) are too slow for real-time tracking
 *
 * Returns:
 * - canUseGPU: true if GPU is compatible with MediaPipe GPU delegate
 * - renderer: GPU renderer name (e.g., "ANGLE (AMD Radeon RX 6800)")
 * - reason: Human-readable explanation of the decision
 */
function detectGPUCapability(): { canUseGPU: boolean; renderer: string; reason: string } {
  const result = { canUseGPU: false, renderer: 'unknown', reason: '' };

  const canvas = document.createElement('canvas');
  let gl: WebGL2RenderingContext | null = null;

  try {
    gl = canvas.getContext('webgl2', {
      failIfMajorPerformanceCaveat: true,
      powerPreference: 'high-performance',
    });
  } catch {
    result.reason = 'WebGL2 context creation failed';
    return result;
  }

  if (!gl) {
    // Retry without failIfMajorPerformanceCaveat
    // Some GPUs (RDNA3, etc) report performance caveat even though they're fast
    try { gl = canvas.getContext('webgl2'); } catch { /* ignore */ }
    if (!gl) {
      result.reason = 'WebGL 2.0 not available';
      return result;
    }
    // WebGL2 available but with performance caveat - still try to use GPU
    // We'll validate renderer name below to filter out software renderers
  }

  // Try WEBGL_debug_renderer_info first, fall back to gl.RENDERER
  // Firefox deprecated the extension and returns privacy-approximated names
  const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
  if (debugInfo) {
    result.renderer = gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL) || 'unknown';
  } else {
    result.renderer = gl.getParameter(gl.RENDERER) || 'unknown';
  }

  // Release WebGL context before MediaPipe init to avoid resource conflicts
  const loseCtx = gl.getExtension('WEBGL_lose_context');
  if (loseCtx) loseCtx.loseContext();

  // Firefox privacy: renderer names ending with "or similar" are approximations
  // e.g., RX 7900 XTX (RDNA3) is reported as "Radeon R9 200 Series ... or similar"
  // In this case, only apply blocklist (software renderers), skip warnlist (legacy GPUs)
  // because the reported name does not reflect the actual hardware.
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
  result.reason = isApproximated
    ? `GPU allowed (Firefox privacy-approximated name): ${result.renderer}`
    : `Compatible GPU: ${result.renderer}`;
  return result;
}

/**
 * Safe division with fallback for degenerate eye geometry.
 *
 * MediaPipe landmarks are normalized to [0,1] range. When eye is closed or
 * landmarks are unstable, denominators can approach zero, causing:
 * - Division by zero (Infinity)
 * - Extreme ratios that break screen coordinate mapping
 *
 * We use ZERO_DIVISION_EPSILON = 0.0001 because:
 * - Typical eye width in normalized coords is ~0.05-0.1
 * - Values < 0.0001 indicate degenerate geometry (closed eye, detection error)
 * - Fallback to 0.5 (screen center) is the safest neutral position
 */
function safeRatio(numerator: number, denominator: number, fallback = 0.5): number {
  if (Math.abs(denominator) < ZERO_DIVISION_EPSILON) return fallback;
  const r = numerator / denominator;
  return Number.isFinite(r) ? r : fallback;
}

/**
 * Converts MediaPipe face landmarks to gaze coordinates.
 *
 * Algorithm:
 * 1. Extract iris centers (landmarks 468, 473) for both eyes
 * 2. Compute gaze ratio: (iris - outer_corner) / (inner_corner - outer_corner)
 * 3. Average left/right eyes to get binocular gaze direction
 * 4. Apply exponential smoothing to reduce jitter (alpha = 0.4-0.5)
 * 5. Map normalized [0,1] coords to screen pixels
 * 6. Detect fixation: distance from center < threshold
 *
 * MediaPipe 478-point model landmark indices:
 * - Right iris center: 468
 * - Left iris center: 473
 * - Eye corners: used to normalize iris position within eye bounds
 */
function processLandmarks(
  lm: { x: number; y: number }[],
  prevGaze: { x: number; y: number },
  alpha: number,
): { gazeData: GazeData; smoothX: number; smoothY: number } | null {
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

  const rawX = (rightGazeX + leftGazeX) / 2;
  const rawY = (rightGazeY + leftGazeY) / 2;

  const smoothX = prevGaze.x * (1 - alpha) + rawX * alpha;
  const smoothY = prevGaze.y * (1 - alpha) + rawY * alpha;

  const screenW = window.innerWidth;
  const screenH = window.innerHeight;
  const clampedX = Math.max(0, Math.min(screenW - 1, Math.round((1 - smoothX) * screenW)));
  const clampedY = Math.max(0, Math.min(screenH - 1, Math.round(smoothY * screenH)));

  const centerDist = Math.sqrt((smoothX - 0.5) ** 2 + (smoothY - 0.5) ** 2);

  return {
    gazeData: {
      x: clampedX,
      y: clampedY,
      confidence: FACE_DETECTED_CONFIDENCE,
      fixated: centerDist < FIXATION_CENTER_DISTANCE_THRESHOLD,
    },
    smoothX,
    smoothY,
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
// Detection loop config — unifies GPU and CPU loops (DRY)
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
  prevGazeRef: React.MutableRefObject<{ x: number; y: number }>;
  cancelledRef: React.MutableRefObject<boolean>;
  lastTimestampRef: React.MutableRefObject<number>;
  lastVideoTimeRef: React.MutableRefObject<number>;
}

interface DetectionConfig {
  interval: number;
  maxErrors: number;
  alpha: number;
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
          const processed = processLandmarks(
            result.faceLandmarks[0],
            refs.prevGazeRef.current,
            config.alpha,
          );
          if (processed) {
            refs.errorCountRef.current = 0;
            refs.prevGazeRef.current = { x: processed.smoothX, y: processed.smoothY };
            config.onResult(processed.gazeData);
            dispatchGaze(processed.gazeData);
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
  const prevGazeRef = useRef<{ x: number; y: number }>({ x: 0.5, y: 0.5 });
  const offCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const offCtxRef = useRef<CanvasRenderingContext2D | null>(null);
  const delegateModeRef = useRef<DelegateMode>('cpu');
  const streamRef = useRef<MediaStream | null>(null);
  const cancelledRef = useRef(false);
  const filesetRef = useRef<VisionFileset | null>(null);

  const [gaze, setGaze] = useState<GazeData | null>(null);
  const [status, setStatus] = useState<TrackerStatus>('loading');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [delegateLabel, setDelegateLabel] = useState<string>('');

  const loopRefs: DetectionLoopRefs = useMemo(
    () => ({
      landmarkerRef,
      timerRef,
      errorCountRef,
      prevGazeRef,
      cancelledRef,
      lastTimestampRef,
      lastVideoTimeRef,
    }),
    [] // Refs never change, so empty dependency array
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
      alpha: CPU_ALPHA,
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
      alpha: GPU_ALPHA,
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
        const gpuInfo = detectGPUCapability();
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

            // Smoke test: Verify GPU delegate actually works at runtime
            //
            // Why we use detect() on a VIDEO-mode landmarker:
            // - MediaPipe's detect() is a convenience method that works in any mode
            // - It internally processes the image as a single frame (IMAGE mode behavior)
            // - detectForVideo() requires a live video stream, which we don't have yet
            // - This tests GPU initialization without requiring camera access first
            //
            // Note: Some GPUs pass createFromOptions() but crash on first detect() call
            // (e.g., GCN 1.0 GPUs with certain driver versions). This catches those cases.
            const testCanvas = document.createElement('canvas');
            testCanvas.width = 320;
            testCanvas.height = 240;
            const testCtx = testCanvas.getContext('2d');
            if (testCtx) testCtx.fillRect(0, 0, 320, 240);
            landmarker.detect(testCanvas);
            console.debug('GPU smoke test passed.');
            useGpu = true;
          } catch (err) {
            console.warn('GPU delegate failed during init/smoke test, falling back to CPU:', err);
            try { landmarker!.close(); } catch { /* ignore */ }
            landmarker = await createLandmarker(fileset, 'CPU', 'IMAGE');
            if (cancelledRef.current) { landmarker.close(); return; }
            delegateModeRef.current = 'cpu-fallback';
            setDelegateLabel('CPU fallback');
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
        const cameraWidth = useGpu ? 640 : 320;
        const cameraHeight = useGpu ? 480 : 240;
        const stream = await navigator.mediaDevices.getUserMedia({
          video: { width: { ideal: cameraWidth }, height: { ideal: cameraHeight }, facingMode: 'user' },
        });
        if (cancelledRef.current) { stream.getTracks().forEach(t => t.stop()); return; }
        streamRef.current = stream;

        const video = videoRef.current;
        if (!video) {
          // Fix stream leak: stop tracks if video element is gone
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

  const fpsLabel = delegateModeRef.current === 'gpu' ? '~30fps' : '~4fps';

  return { videoRef, gaze, status, errorMsg, delegateLabel, fpsLabel };
}
