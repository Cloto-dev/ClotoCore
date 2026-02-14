import sys
import json
import time
import math

# Try to import vision libraries, fallback to mock mode if unavailable
try:
    import cv2
    import mediapipe as mp
    import numpy as np
    REAL_MODE = True
except ImportError as e:
    print(f"⚠️ Vision libraries not available: {e}", file=sys.stderr)
    print("👁️ Falling back to Mock Mode. Install: pip install opencv-python mediapipe", file=sys.stderr)
    REAL_MODE = False

# Exiv Plugin Interface
EXIV_MANIFEST = {
    "id": "python.gaze",
    "name": "Webcam Gaze Tracker",
    "description": "Real-time eye tracking via webcam (MediaPipe Face Mesh). Click eye icon to toggle.",
    "version": "1.0.0",
    "category": "Tool",
    "service_type": "Vision",
    "tags": ["#VISION", "#EYE-TRACKING", "#WEBCAM", "#MEDIAPIPE"],
    "required_permissions": ["VisionRead", "CameraAccess"],
    "action_icon": "Eye",
    "action_target": "api:toggle"
}

# Gaze State
is_tracking = False
camera = None
face_mesh = None

def on_action_toggle(params):
    global is_tracking
    is_tracking = not is_tracking
    status = "ON" if is_tracking else "OFF"
    mode = "REAL" if REAL_MODE else "MOCK"
    emit_event("SystemNotification", f"Gaze Tracking ({mode}) is now {status}")
    return {"status": status, "mode": mode}

def setup():
    """Called by bridge_runtime when script starts"""
    global camera, face_mesh

    if REAL_MODE:
        try:
            # Initialize MediaPipe Face Landmarker (new API for 0.10.x)
            from mediapipe.tasks import python
            from mediapipe.tasks.python import vision
            import os

            # Get model path (relative to this script)
            script_dir = os.path.dirname(os.path.abspath(__file__))
            model_path = os.path.join(script_dir, 'models', 'face_landmarker.task')

            base_options = python.BaseOptions(model_asset_path=model_path)
            options = vision.FaceLandmarkerOptions(
                base_options=base_options,
                num_faces=1,
                min_face_detection_confidence=0.5,
                min_face_presence_confidence=0.5,
                min_tracking_confidence=0.5,
                output_face_blendshapes=False,
                output_facial_transformation_matrixes=False
            )
            face_mesh = vision.FaceLandmarker.create_from_options(options)

            # Initialize webcam
            camera = cv2.VideoCapture(0)
            if not camera.isOpened():
                raise RuntimeError("Failed to open webcam")

            # Set camera properties
            camera.set(cv2.CAP_PROP_FRAME_WIDTH, 640)
            camera.set(cv2.CAP_PROP_FRAME_HEIGHT, 480)
            camera.set(cv2.CAP_PROP_FPS, 30)

            print("👁️ Gaze Tracker: Ready in REAL MODE (MediaPipe FaceLandmarker). Standing by...", file=sys.stderr)
        except Exception as e:
            print(f"❌ Failed to initialize camera: {e}", file=sys.stderr)
            print("👁️ Falling back to Mock Mode", file=sys.stderr)
            globals()['REAL_MODE'] = False

    if not REAL_MODE:
        print("👁️ Gaze Tracker: Standby mode (no camera). Waiting for camera connection...", file=sys.stderr)

    if REAL_MODE:
        import threading
        threading.Thread(target=real_tracking_loop, daemon=True).start()

def real_tracking_loop():
    """Real webcam-based eye tracking loop"""
    global is_tracking, camera, face_mesh

    # Screen dimensions (will be calibrated)
    screen_w, screen_h = 1920, 1080

    # Eye landmark indices (MediaPipe Face Mesh)
    LEFT_EYE_IRIS = [468, 469, 470, 471, 472]  # Left iris landmarks
    RIGHT_EYE_IRIS = [473, 474, 475, 476, 477]  # Right iris landmarks

    while True:
        if not is_tracking:
            time.sleep(0.1)
            continue

        try:
            ret, frame = camera.read()
            if not ret:
                emit_event("SystemNotification", "Camera read failed")
                time.sleep(0.5)
                continue

            # Convert BGR to RGB and create MediaPipe Image
            rgb_frame = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
            from mediapipe import Image, ImageFormat
            mp_image = Image(image_format=ImageFormat.SRGB, data=rgb_frame)

            # Detect face landmarks
            results = face_mesh.detect(mp_image)

            if results.face_landmarks:
                # Get first face landmarks
                landmarks = results.face_landmarks[0]

                # Get iris centers (landmarks 468-477 are iris points)
                left_iris_x = np.mean([landmarks[i].x for i in LEFT_EYE_IRIS])
                left_iris_y = np.mean([landmarks[i].y for i in LEFT_EYE_IRIS])
                right_iris_x = np.mean([landmarks[i].x for i in RIGHT_EYE_IRIS])
                right_iris_y = np.mean([landmarks[i].y for i in RIGHT_EYE_IRIS])

                # Average both eyes
                gaze_x = (left_iris_x + right_iris_x) / 2
                gaze_y = (left_iris_y + right_iris_y) / 2

                # Map to screen coordinates (normalized 0-1 to screen pixels)
                # Invert X for natural mapping (look left = left side of screen)
                screen_x = int((1 - gaze_x) * screen_w)
                screen_y = int(gaze_y * screen_h)

                # Clamp to screen bounds
                screen_x = max(0, min(screen_w - 1, screen_x))
                screen_y = max(0, min(screen_h - 1, screen_y))

                # Calculate confidence based on detection
                confidence = 0.85  # High confidence when face detected

                # Check if gaze is relatively stable (fixation detection)
                # Simple heuristic: near center is fixated
                center_dist = math.sqrt((gaze_x - 0.5)**2 + (gaze_y - 0.5)**2)
                fixated = center_dist < 0.1

                emit_event("GazeUpdated", {
                    "x": screen_x,
                    "y": screen_y,
                    "confidence": confidence,
                    "fixated": fixated,
                    "raw_x": float(gaze_x),
                    "raw_y": float(gaze_y)
                })
            else:
                # No face detected
                emit_event("GazeUpdated", {
                    "x": screen_w // 2,
                    "y": screen_h // 2,
                    "confidence": 0.0,
                    "fixated": False
                })

            time.sleep(0.033)  # ~30 FPS

        except Exception as e:
            print(f"❌ Tracking error: {e}", file=sys.stderr)
            time.sleep(0.5)

def think(params):
    mode = "REAL (MediaPipe)" if REAL_MODE else "MOCK"
    status = "ACTIVE" if is_tracking else "STANDBY"
    return f"Gaze tracking: {mode} mode, {status}"

def cleanup():
    """Called when plugin shuts down"""
    global camera
    if camera is not None:
        camera.release()
        print("👁️ Camera released", file=sys.stderr)
