import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { useGazeDetection } from '../useGazeDetection';

// Mock MediaPipe modules
vi.mock('@mediapipe/tasks-vision', () => ({
  FaceLandmarker: {
    createFromOptions: vi.fn(() =>
      Promise.resolve({
        detect: vi.fn(() => ({ faceLandmarks: [], faceBlendshapes: [], facialTransformationMatrixes: [] })),
        detectForVideo: vi.fn(() => ({ faceLandmarks: [], faceBlendshapes: [], facialTransformationMatrixes: [] })),
        close: vi.fn(),
      })
    ),
  },
  FilesetResolver: {
    forVisionTasks: vi.fn(() => Promise.resolve({})),
  },
}));

describe('useGazeDetection', () => {
  beforeEach(() => {
    vi.clearAllMocks();

    // Reset getUserMedia mock (already defined in setup.ts)
    vi.mocked(navigator.mediaDevices.getUserMedia).mockResolvedValue({
      getTracks: () => [{ stop: vi.fn() }],
      getVideoTracks: () => [{ stop: vi.fn() }],
      getAudioTracks: () => [],
    } as unknown as MediaStream);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('Initialization', () => {
    it('should initialize with loading status', () => {
      const { result } = renderHook(() => useGazeDetection());

      expect(result.current.status).toBe('loading');
      expect(result.current.gaze).toBeNull();
      expect(result.current.errorMsg).toBeNull();
    });

    it('should provide a video ref', () => {
      const { result } = renderHook(() => useGazeDetection());

      expect(result.current.videoRef).toBeDefined();
      expect(result.current.videoRef.current).toBeNull(); // Not attached to DOM
    });

    it('should request camera access after initialization', async () => {
      const { result } = renderHook(() => useGazeDetection());

      await waitFor(
        () => {
          expect(result.current.status).toBe('requesting');
        },
        { timeout: 3000 }
      );
    });
  });

  describe('GPU Detection', () => {
    it('should detect GPU capability', () => {
      const { result } = renderHook(() => useGazeDetection());

      // GPU detection runs synchronously at start
      // We can't easily test the internal state, but we can verify no crashes
      expect(result.current.status).toBe('loading');
    });

    it('should fall back to CPU if GPU fails', async () => {
      // Mock GPU failure
      const { FaceLandmarker } = await import('@mediapipe/tasks-vision');
      vi.mocked(FaceLandmarker.createFromOptions).mockRejectedValueOnce(new Error('GPU init failed'));

      const { result } = renderHook(() => useGazeDetection());

      await waitFor(
        () => {
          expect(result.current.delegateLabel).toContain('CPU');
        },
        { timeout: 5000 }
      );
    });
  });

  describe('Error Handling', () => {
    it('should handle camera permission denial', async () => {
      const error = new Error('Camera denied');
      (error as any).name = 'NotAllowedError';

      vi.mocked(navigator.mediaDevices.getUserMedia).mockRejectedValueOnce(error);

      const { result } = renderHook(() => useGazeDetection());

      await waitFor(
        () => {
          expect(result.current.status).toBe('denied');
          expect(result.current.errorMsg).toContain('denied');
        },
        { timeout: 5000 }
      );
    });

    it('should handle generic initialization errors', async () => {
      const { FilesetResolver } = await import('@mediapipe/tasks-vision');
      vi.mocked(FilesetResolver.forVisionTasks).mockRejectedValueOnce(new Error('WASM load failed'));

      const { result } = renderHook(() => useGazeDetection());

      await waitFor(
        () => {
          expect(result.current.status).toBe('error');
          expect(result.current.errorMsg).toBeTruthy();
        },
        { timeout: 5000 }
      );
    });
  });

  describe('Cleanup', () => {
    it('should clean up resources on unmount', async () => {
      const stopMock = vi.fn();
      const closeMock = vi.fn();

      vi.mocked(navigator.mediaDevices.getUserMedia).mockResolvedValueOnce({
        getTracks: () => [{ stop: stopMock }],
        getVideoTracks: () => [],
        getAudioTracks: () => [],
      } as unknown as MediaStream);

      const { FaceLandmarker } = await import('@mediapipe/tasks-vision');
      vi.mocked(FaceLandmarker.createFromOptions).mockResolvedValueOnce({
        detect: vi.fn(),
        detectForVideo: vi.fn(),
        close: closeMock,
      } as any);

      const { unmount } = renderHook(() => useGazeDetection());

      // Wait for initialization
      await waitFor(() => {}, { timeout: 3000 });

      unmount();

      // Verify cleanup (may be called multiple times due to strict mode)
      await waitFor(() => {
        expect(stopMock).toHaveBeenCalled();
      });
    });
  });

  describe('FPS Label', () => {
    it('should return correct FPS label for CPU mode', () => {
      const { result } = renderHook(() => useGazeDetection());

      // Default is CPU mode
      expect(result.current.fpsLabel).toBe('~4fps');
    });
  });

  describe('Delegate Label', () => {
    it('should start with empty delegate label', () => {
      const { result } = renderHook(() => useGazeDetection());

      expect(result.current.delegateLabel).toBe('');
    });
  });
});
