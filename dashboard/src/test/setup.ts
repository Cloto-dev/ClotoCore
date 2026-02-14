import { expect, afterEach, vi } from 'vitest';
import { cleanup } from '@testing-library/react';
import '@testing-library/jest-dom/vitest';

// Cleanup after each test
afterEach(() => {
  cleanup();
});

// Mock MediaPipe and browser APIs
global.HTMLMediaElement.prototype.play = vi.fn(() => Promise.resolve());
global.HTMLMediaElement.prototype.pause = vi.fn();

// Mock canvas
HTMLCanvasElement.prototype.getContext = vi.fn(() => ({
  fillRect: vi.fn(),
  clearRect: vi.fn(),
  getImageData: vi.fn(),
  putImageData: vi.fn(),
  createImageData: vi.fn(),
  setTransform: vi.fn(),
  drawImage: vi.fn(),
  save: vi.fn(),
  fillText: vi.fn(),
  restore: vi.fn(),
  beginPath: vi.fn(),
  moveTo: vi.fn(),
  lineTo: vi.fn(),
  closePath: vi.fn(),
  stroke: vi.fn(),
  translate: vi.fn(),
  scale: vi.fn(),
  rotate: vi.fn(),
  arc: vi.fn(),
  fill: vi.fn(),
  measureText: vi.fn(() => ({ width: 0 })),
  transform: vi.fn(),
  rect: vi.fn(),
  clip: vi.fn(),
})) as any;

// Mock WebGL context
HTMLCanvasElement.prototype.getContext = vi.fn((contextType: string) => {
  if (contextType === 'webgl2') {
    return {
      getExtension: vi.fn((name: string) => {
        if (name === 'WEBGL_debug_renderer_info') {
          return {
            UNMASKED_RENDERER_WEBGL: 37446,
          };
        }
        if (name === 'WEBGL_lose_context') {
          return { loseContext: vi.fn() };
        }
        return null;
      }),
      getParameter: vi.fn((param: number) => {
        if (param === 37446) return 'Mock GPU Renderer';
        return null;
      }),
    } as any;
  }
  return null;
}) as any;

// Mock MediaDevices
Object.defineProperty(navigator, 'mediaDevices', {
  value: {
    getUserMedia: vi.fn(() =>
      Promise.resolve({
        getTracks: () => [],
        getVideoTracks: () => [],
        getAudioTracks: () => [],
      } as MediaStream)
    ),
  },
  writable: true,
});
