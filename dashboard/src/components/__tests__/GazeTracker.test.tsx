import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { GazeTracker } from '../GazeTracker';

// Mock the hook
vi.mock('../../hooks/useGazeDetection', () => ({
  useGazeDetection: vi.fn(() => ({
    videoRef: { current: null },
    gaze: null,
    status: 'loading',
    errorMsg: null,
    delegateLabel: '',
    fpsLabel: '~4fps',
  })),
}));

describe('GazeTracker Component', () => {
  it('should render loading state', () => {
    render(<GazeTracker />);
    expect(screen.getByText(/Loading MediaPipe model/i)).toBeInTheDocument();
  });

  it('should render requesting state', async () => {
    const { useGazeDetection } = await import('../../hooks/useGazeDetection');
    vi.mocked(useGazeDetection).mockReturnValueOnce({
      videoRef: { current: null },
      gaze: null,
      status: 'requesting',
      errorMsg: null,
      delegateLabel: '',
      fpsLabel: '~4fps',
    });

    render(<GazeTracker />);
    expect(screen.getByText(/Requesting camera access/i)).toBeInTheDocument();
  });

  it('should render active state with gaze data', async () => {
    const { useGazeDetection } = await import('../../hooks/useGazeDetection');
    vi.mocked(useGazeDetection).mockReturnValueOnce({
      videoRef: { current: null },
      gaze: { x: 100, y: 200, confidence: 0.85, fixated: false },
      status: 'active',
      errorMsg: null,
      delegateLabel: 'GPU',
      fpsLabel: '~30fps',
    });

    render(<GazeTracker />);
    expect(screen.getByText(/EYE TRACKING \(GPU ~30fps\)/i)).toBeInTheDocument();
    expect(screen.getByText(/\(100, 200\)/i)).toBeInTheDocument();
    expect(screen.getByText(/Conf: 85%/i)).toBeInTheDocument();
  });

  it('should render fixated indicator when fixated', async () => {
    const { useGazeDetection } = await import('../../hooks/useGazeDetection');
    vi.mocked(useGazeDetection).mockReturnValueOnce({
      videoRef: { current: null },
      gaze: { x: 100, y: 200, confidence: 0.85, fixated: true },
      status: 'active',
      errorMsg: null,
      delegateLabel: 'CPU',
      fpsLabel: '~4fps',
    });

    render(<GazeTracker />);
    expect(screen.getByText(/FIXATED/i)).toBeInTheDocument();
  });

  it('should render error state', async () => {
    const { useGazeDetection } = await import('../../hooks/useGazeDetection');
    vi.mocked(useGazeDetection).mockReturnValueOnce({
      videoRef: { current: null },
      gaze: null,
      status: 'error',
      errorMsg: 'Initialization failed',
      delegateLabel: '',
      fpsLabel: '~4fps',
    });

    render(<GazeTracker />);
    expect(screen.getByText(/Initialization failed/i)).toBeInTheDocument();
  });

  it('should render denied state', async () => {
    const { useGazeDetection } = await import('../../hooks/useGazeDetection');
    vi.mocked(useGazeDetection).mockReturnValueOnce({
      videoRef: { current: null },
      gaze: null,
      status: 'denied',
      errorMsg: 'Camera access denied',
      delegateLabel: '',
      fpsLabel: '~4fps',
    });

    render(<GazeTracker />);
    expect(screen.getByText(/Camera access denied/i)).toBeInTheDocument();
  });

  it('should render stopped state', async () => {
    const { useGazeDetection } = await import('../../hooks/useGazeDetection');
    vi.mocked(useGazeDetection).mockReturnValueOnce({
      videoRef: { current: null },
      gaze: null,
      status: 'stopped',
      errorMsg: 'Detection stopped',
      delegateLabel: '',
      fpsLabel: '~4fps',
    });

    render(<GazeTracker />);
    expect(screen.getByText(/EYE TRACKING STOPPED/i)).toBeInTheDocument();
  });

  it('should show video element only when active', async () => {
    const { useGazeDetection } = await import('../../hooks/useGazeDetection');
    vi.mocked(useGazeDetection).mockReturnValueOnce({
      videoRef: { current: null },
      gaze: { x: 100, y: 200, confidence: 0.85, fixated: false },
      status: 'active',
      errorMsg: null,
      delegateLabel: 'GPU',
      fpsLabel: '~30fps',
    });

    const { container } = render(<GazeTracker />);
    const video = container.querySelector('video');
    expect(video).toBeInTheDocument();
    expect(video).toHaveClass('w-32', 'h-24');
  });

  it('should hide video element when not active', async () => {
    const { useGazeDetection } = await import('../../hooks/useGazeDetection');
    vi.mocked(useGazeDetection).mockReturnValueOnce({
      videoRef: { current: null },
      gaze: null,
      status: 'loading',
      errorMsg: null,
      delegateLabel: '',
      fpsLabel: '~4fps',
    });

    const { container } = render(<GazeTracker />);
    const video = container.querySelector('video');
    expect(video).toBeInTheDocument();
    expect(video).toHaveClass('hidden');
  });
});
