"""
Gaze tracking engine using MediaPipe FaceLandmarker + One Euro Filter.

Runs camera capture and face landmark detection in a background thread.
MCP tool handlers read the latest gaze result from shared state.
"""

import math
import threading
import time
from dataclasses import dataclass, field

import cv2
import mediapipe as mp
import numpy as np

# ============================================================
# Constants
# ============================================================

CAMERA_WIDTH = 640
CAMERA_HEIGHT = 480
CAMERA_FPS = 15

# MediaPipe 478-point model landmark indices
RIGHT_IRIS_CENTER = 468
LEFT_IRIS_CENTER = 473
RIGHT_EYE_OUTER = 33
RIGHT_EYE_INNER = 133
LEFT_EYE_OUTER = 263
LEFT_EYE_INNER = 362
RIGHT_EYE_TOP = 159
RIGHT_EYE_BOTTOM = 145
LEFT_EYE_TOP = 386
LEFT_EYE_BOTTOM = 374
MIN_LANDMARKS_FOR_IRIS = 474

FACE_CONFIDENCE_THRESHOLD = 0.5
ZERO_DIVISION_EPSILON = 0.0001

# One Euro Filter parameters (same as TypeScript version)
ONE_EURO_MINCUTOFF = 1.0
ONE_EURO_BETA = 0.5
ONE_EURO_DCUTOFF = 1.0

# MediaPipe model URL
FACE_MODEL_URL = (
    "https://storage.googleapis.com/mediapipe-models/"
    "face_landmarker/face_landmarker/float16/1/face_landmarker.task"
)


# ============================================================
# One Euro Filter (Casiez et al., CHI 2012)
# ============================================================

class _LowPassFilter:
    __slots__ = ("_s", "_raw", "_initialized")

    def __init__(self) -> None:
        self._s = 0.0
        self._raw = 0.0
        self._initialized = False

    @property
    def has_last_raw(self) -> bool:
        return self._initialized

    @property
    def last_raw(self) -> float:
        return self._raw

    def filter(self, value: float, alpha: float) -> float:
        self._raw = value
        if self._initialized:
            self._s = alpha * value + (1.0 - alpha) * self._s
        else:
            self._s = value
            self._initialized = True
        return self._s


class OneEuroFilter:
    __slots__ = ("_freq", "_mincutoff", "_beta", "_dcutoff",
                 "_x_filt", "_dx_filt", "_last_time")

    def __init__(
        self,
        freq: float = 15.0,
        mincutoff: float = ONE_EURO_MINCUTOFF,
        beta: float = ONE_EURO_BETA,
        dcutoff: float = ONE_EURO_DCUTOFF,
    ) -> None:
        self._freq = freq
        self._mincutoff = mincutoff
        self._beta = beta
        self._dcutoff = dcutoff
        self._x_filt = _LowPassFilter()
        self._dx_filt = _LowPassFilter()
        self._last_time = -1.0

    def _alpha(self, cutoff: float) -> float:
        te = 1.0 / self._freq
        tau = 1.0 / (2.0 * math.pi * cutoff)
        return 1.0 / (1.0 + tau / te)

    def filter(self, value: float, timestamp: float) -> float:
        if self._last_time >= 0:
            dt = timestamp - self._last_time
            if dt > 0:
                self._freq = 1.0 / dt
        self._last_time = timestamp

        dvalue = (
            (value - self._x_filt.last_raw) * self._freq
            if self._x_filt.has_last_raw
            else 0.0
        )
        edvalue = self._dx_filt.filter(dvalue, self._alpha(self._dcutoff))
        cutoff = self._mincutoff + self._beta * abs(edvalue)
        return self._x_filt.filter(value, self._alpha(cutoff))


# ============================================================
# Gaze Result
# ============================================================

@dataclass
class GazeResult:
    """Normalized gaze coordinates. No screen mapping — consumers decide."""
    gaze_x: float = 0.5       # [0,1] horizontal (0=left, 1=right)
    gaze_y: float = 0.5       # [0,1] vertical (0=up, 1=down)
    face_detected: bool = False
    confidence: float = 0.0
    timestamp: float = 0.0


# ============================================================
# Gaze Engine
# ============================================================

def _safe_ratio(numerator: float, denominator: float, fallback: float = 0.5) -> float:
    if abs(denominator) < ZERO_DIVISION_EPSILON:
        return fallback
    r = numerator / denominator
    return r if math.isfinite(r) else fallback


def _extract_gaze_ratios(
    landmarks: list,
) -> tuple[float, float] | None:
    """Extract normalized gaze ratios from MediaPipe face landmarks."""
    if len(landmarks) < MIN_LANDMARKS_FOR_IRIS:
        return None

    right_gaze_x = _safe_ratio(
        landmarks[RIGHT_IRIS_CENTER].x - landmarks[RIGHT_EYE_OUTER].x,
        landmarks[RIGHT_EYE_INNER].x - landmarks[RIGHT_EYE_OUTER].x,
    )
    left_gaze_x = _safe_ratio(
        landmarks[LEFT_IRIS_CENTER].x - landmarks[LEFT_EYE_OUTER].x,
        landmarks[LEFT_EYE_INNER].x - landmarks[LEFT_EYE_OUTER].x,
    )
    right_gaze_y = _safe_ratio(
        landmarks[RIGHT_IRIS_CENTER].y - landmarks[RIGHT_EYE_TOP].y,
        landmarks[RIGHT_EYE_BOTTOM].y - landmarks[RIGHT_EYE_TOP].y,
    )
    left_gaze_y = _safe_ratio(
        landmarks[LEFT_IRIS_CENTER].y - landmarks[LEFT_EYE_TOP].y,
        landmarks[LEFT_EYE_BOTTOM].y - landmarks[LEFT_EYE_TOP].y,
    )

    # Left eye X ratio runs opposite to right eye — flip to align
    raw_x = (right_gaze_x + (1.0 - left_gaze_x)) / 2.0
    raw_y = (right_gaze_y + left_gaze_y) / 2.0

    return raw_x, raw_y


MODEL_DOWNLOAD_TIMEOUT = 30  # seconds
MODEL_DOWNLOAD_RETRIES = 3


class GazeEngine:
    """Background gaze tracking engine. Thread-safe read access to latest result."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._result = GazeResult()
        self._running = False
        self._error: str | None = None  # bug-119: propagate thread errors
        self._thread: threading.Thread | None = None
        self._stop_event = threading.Event()
        self._fps = 0.0
        self._camera_resolution = (0, 0)

    def start(self) -> str:
        """Start camera capture and gaze tracking. Returns status message."""
        with self._lock:  # bug-122: protect is_running reads
            if self._running:
                return "already_running"

        self._error = None
        self._stop_event.clear()
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        return "started"

    def stop(self) -> str:
        """Stop camera capture and release resources."""
        with self._lock:
            if not self._running:
                return "not_running"

        self._stop_event.set()
        if self._thread is not None:
            self._thread.join(timeout=5.0)
            self._thread = None

        with self._lock:
            self._running = False
            self._result = GazeResult()

        return "stopped"

    @property
    def is_running(self) -> bool:
        with self._lock:  # bug-122: thread-safe read
            return self._running

    @property
    def error(self) -> str | None:
        """Returns error message if background thread crashed."""
        with self._lock:
            return self._error

    def get_gaze(self) -> GazeResult:
        with self._lock:
            return GazeResult(
                gaze_x=self._result.gaze_x,
                gaze_y=self._result.gaze_y,
                face_detected=self._result.face_detected,
                confidence=self._result.confidence,
                timestamp=self._result.timestamp,
            )

    def get_status(self) -> dict:
        with self._lock:
            return {
                "running": self._running,
                "fps": round(self._fps, 1),
                "camera_resolution": list(self._camera_resolution),
                "face_detected": self._result.face_detected,
                "error": self._error,
            }

    def _run(self) -> None:
        """Background thread: camera capture → MediaPipe → One Euro Filter."""
        cap = None
        landmarker = None

        try:
            # Initialize MediaPipe FaceLandmarker
            model_data = self._download_model()
            base_options = mp.tasks.BaseOptions(
                model_asset_path=None,
                model_asset_buffer=model_data,
            )
            options = mp.tasks.vision.FaceLandmarkerOptions(
                base_options=base_options,
                running_mode=mp.tasks.vision.RunningMode.IMAGE,
                num_faces=1,
                min_face_detection_confidence=FACE_CONFIDENCE_THRESHOLD,
                min_face_presence_confidence=FACE_CONFIDENCE_THRESHOLD,
                min_tracking_confidence=FACE_CONFIDENCE_THRESHOLD,
                output_face_blendshapes=False,
                output_facial_transformation_matrixes=False,
            )
            landmarker = mp.tasks.vision.FaceLandmarker.create_from_options(options)

            # Open camera
            cap = cv2.VideoCapture(0)
            cap.set(cv2.CAP_PROP_FRAME_WIDTH, CAMERA_WIDTH)
            cap.set(cv2.CAP_PROP_FRAME_HEIGHT, CAMERA_HEIGHT)
            cap.set(cv2.CAP_PROP_FPS, CAMERA_FPS)

            if not cap.isOpened():
                raise RuntimeError("Failed to open camera")

            actual_w = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
            actual_h = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))

            with self._lock:
                self._running = True
                self._camera_resolution = (actual_w, actual_h)

            # One Euro Filters (one per axis)
            filter_x = OneEuroFilter(freq=CAMERA_FPS)
            filter_y = OneEuroFilter(freq=CAMERA_FPS)

            frame_count = 0
            fps_start = time.monotonic()
            interval = 1.0 / CAMERA_FPS

            while not self._stop_event.is_set():
                t0 = time.monotonic()
                ret, frame = cap.read()
                if not ret:
                    time.sleep(interval)
                    continue

                # Convert BGR → RGB for MediaPipe
                rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
                mp_image = mp.Image(image_format=mp.ImageFormat.SRGB, data=rgb)
                result = landmarker.detect(mp_image)

                now = time.time()

                if result.face_landmarks and len(result.face_landmarks) > 0:
                    ratios = _extract_gaze_ratios(result.face_landmarks[0])
                    if ratios is not None:
                        raw_x, raw_y = ratios
                        smooth_x = filter_x.filter(raw_x, now)
                        smooth_y = filter_y.filter(raw_y, now)

                        # bug-121: clamp to [0, 1] range
                        clamped_x = max(0.0, min(1.0, smooth_x))
                        clamped_y = max(0.0, min(1.0, smooth_y))

                        with self._lock:
                            self._result = GazeResult(
                                gaze_x=clamped_x,
                                gaze_y=clamped_y,
                                face_detected=True,
                                confidence=0.85,
                                timestamp=now,
                            )
                else:
                    with self._lock:
                        self._result = GazeResult(
                            gaze_x=self._result.gaze_x,
                            gaze_y=self._result.gaze_y,
                            face_detected=False,
                            confidence=0.0,
                            timestamp=now,
                        )

                # FPS calculation
                frame_count += 1
                elapsed_fps = time.monotonic() - fps_start
                if elapsed_fps >= 1.0:
                    with self._lock:
                        self._fps = frame_count / elapsed_fps
                    frame_count = 0
                    fps_start = time.monotonic()

                # Maintain target frame rate
                elapsed = time.monotonic() - t0
                sleep_time = interval - elapsed
                if sleep_time > 0:
                    time.sleep(sleep_time)

        except Exception as e:
            # bug-119: propagate error to caller via shared state
            import sys
            err_msg = f"GazeEngine error: {e}"
            print(err_msg, file=sys.stderr)
            with self._lock:
                self._error = err_msg
        finally:
            if cap is not None:
                cap.release()
            if landmarker is not None:
                landmarker.close()
            with self._lock:
                self._running = False

    @staticmethod
    def _download_model() -> bytes:
        """Download MediaPipe FaceLandmarker model with retry and timeout."""
        import urllib.request
        import sys

        for attempt in range(1, MODEL_DOWNLOAD_RETRIES + 1):
            try:
                print(f"Downloading FaceLandmarker model (attempt {attempt}/{MODEL_DOWNLOAD_RETRIES})...", file=sys.stderr)
                req = urllib.request.Request(FACE_MODEL_URL)
                with urllib.request.urlopen(req, timeout=MODEL_DOWNLOAD_TIMEOUT) as resp:
                    data = resp.read()
                print(f"Model downloaded ({len(data)} bytes)", file=sys.stderr)
                return data
            except Exception as e:
                print(f"Model download failed (attempt {attempt}): {e}", file=sys.stderr)
                if attempt == MODEL_DOWNLOAD_RETRIES:
                    raise RuntimeError(f"Failed to download model after {MODEL_DOWNLOAD_RETRIES} attempts: {e}") from e
                time.sleep(2 * attempt)  # backoff
        raise RuntimeError("Unreachable")
