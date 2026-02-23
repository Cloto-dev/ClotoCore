import { useEffect, useRef } from 'react';
import { useTheme } from '../hooks/useTheme';
import { getDpr } from '../lib/canvasUtils';

interface Point { x: number; y: number; r: number }

/** Accent color for gaze-tracking mode cursor */
const GAZE_COLOR = '#ec4899';

export function CustomCursor() {
  const trailCanvasRef = useRef<HTMLCanvasElement>(null);
  const points = useRef<Point[]>([]);
  const lastPlacedMouse = useRef({ x: -1000, y: -1000 });
  const localMouse = useRef({ x: -1000, y: -1000 });
  const gazeTarget = useRef({ x: -1000, y: -1000 });
  const isGazeActiveRef = useRef(false);
  const gazeTimeoutRef = useRef<number>(0);
  const { colors } = useTheme();
  const brandHexRef = useRef(colors.brandHex);
  brandHexRef.current = colors.brandHex;

  // Hide native cursor globally while this component is mounted
  useEffect(() => {
    document.body.classList.add('neural-cursor-active');
    return () => { document.body.classList.remove('neural-cursor-active'); };
  }, []);

  // Listen for gaze events from GazeTracker via browser CustomEvent (no backend roundtrip)
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      if (detail && typeof detail.x === 'number') {
        gazeTarget.current = { x: detail.x, y: detail.y };
        isGazeActiveRef.current = true;
        clearTimeout(gazeTimeoutRef.current);
        gazeTimeoutRef.current = window.setTimeout(() => {
          isGazeActiveRef.current = false;
        }, 500);
      }
    };
    window.addEventListener('exiv-gaze', handler);
    return () => {
      window.removeEventListener('exiv-gaze', handler);
      clearTimeout(gazeTimeoutRef.current);
    };
  }, []);

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      localMouse.current = { x: e.clientX, y: e.clientY };
      if (!isGazeActiveRef.current) {
        gazeTarget.current = { x: e.clientX, y: e.clientY };
      }
    };
    window.addEventListener('mousemove', handleMouseMove, { passive: true });

    let rafId: number;
    let tctx: CanvasRenderingContext2D | null = null;
    let octx: CanvasRenderingContext2D | null = null;
    let offscreenCanvas: HTMLCanvasElement | null = null;
    let prevMx = -1000;
    let prevMy = -1000;
    let isIdle = false;

    // Dynamic decay state
    let smoothSpeed = 0;           // Exponential moving average of cursor speed (px/frame)
    let decayAccumulator = 0;      // Fractional accumulator for sub-1 decay rates
    let maxTrailLen = 50;          // Dynamic max trail length based on speed

    // Dirty region tracking (previous frame's bounding box to clear)
    let prevDirtyX = 0;
    let prevDirtyY = 0;
    let prevDirtyW = 0;
    let prevDirtyH = 0;

    // Filter throttle: apply goo filter every N frames, reuse cached result otherwise
    let filterFrame = 0;
    let filteredCanvas: HTMLCanvasElement | null = null;
    let filteredCtx: CanvasRenderingContext2D | null = null;
    let prevFilterMx = 0;
    let prevFilterMy = 0;

    const initCanvas = () => {
      const canvas = trailCanvasRef.current;
      if (!canvas) return false;
      tctx = canvas.getContext('2d');
      offscreenCanvas = document.createElement('canvas');
      octx = offscreenCanvas.getContext('2d');
      filteredCanvas = document.createElement('canvas');
      filteredCtx = filteredCanvas.getContext('2d');
      if (!tctx || !octx || !filteredCtx) return false;
      return true;
    };

    const handleResize = () => {
      const canvas = trailCanvasRef.current;
      if (!canvas || !tctx || !octx || !offscreenCanvas || !filteredCanvas || !filteredCtx) return;
      const dpr = getDpr();
      const width = window.innerWidth;
      const height = window.innerHeight;
      canvas.width = width * dpr;
      canvas.height = height * dpr;
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;
      offscreenCanvas.width = 400 * dpr;
      offscreenCanvas.height = 400 * dpr;
      filteredCanvas.width = 400 * dpr;
      filteredCanvas.height = 400 * dpr;
      octx.setTransform(dpr, 0, 0, dpr, 0, 0);
      filteredCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
      tctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };

    window.addEventListener('resize', handleResize);

    // Render at monitor's native refresh rate (up to 120fps via RAF)
    const render = () => {
      rafId = requestAnimationFrame(render);
      if (!tctx && !initCanvas()) return;
      if (!tctx || !octx || !offscreenCanvas) return;
      if (document.hidden) return;

      // Smooth Lerp towards target (Gaze or Mouse)
      const lerpSpeed = isGazeActiveRef.current ? 0.15 : 1.0;
      const targetX = gazeTarget.current.x;
      const targetY = gazeTarget.current.y;

      if (targetX === -1000) return;

      const mx = localMouse.current.x + (targetX - localMouse.current.x) * lerpSpeed;
      const my = localMouse.current.y + (targetY - localMouse.current.y) * lerpSpeed;
      localMouse.current = { x: mx, y: my };

      // Instantaneous speed (px/frame)
      const frameDx = mx - prevMx;
      const frameDy = my - prevMy;
      const frameSpeed = (prevMx === -1000) ? 0 : Math.sqrt(frameDx * frameDx + frameDy * frameDy);

      // Exponential moving average: fast ramp-up, gradual cooldown
      const alpha = frameSpeed > smoothSpeed ? 0.3 : 0.05;
      smoothSpeed += (frameSpeed - smoothSpeed) * alpha;

      // Idle detection: skip trail rendering when cursor is stationary and trail is gone
      // but always draw the cursor dot itself
      const moved = Math.abs(mx - prevMx) > 0.5 || Math.abs(my - prevMy) > 0.5;
      if (!moved && points.current.length === 0) {
        if (!isIdle) {
          if (prevDirtyW > 0) {
            tctx.clearRect(prevDirtyX, prevDirtyY, prevDirtyW, prevDirtyH);
          }
          tctx.clearRect(Math.floor(mx - 220), Math.floor(my - 220), 440, 440);
          octx.clearRect(0, 0, 400, 400);
          octx.save();
          octx.translate(200 - mx, 200 - my);
          octx.fillStyle = isGazeActiveRef.current ? GAZE_COLOR : brandHexRef.current;
          octx.globalAlpha = 1;
          octx.beginPath();
          octx.arc(mx, my, isGazeActiveRef.current ? 6 : 4, 0, Math.PI * 2);
          octx.fill();
          octx.restore();
          tctx.save();
          tctx.filter = 'url(#goo-cursor)';
          tctx.drawImage(offscreenCanvas, mx - 200, my - 200, 400, 400);
          tctx.restore();
          prevDirtyX = Math.floor(mx - 220);
          prevDirtyY = Math.floor(my - 220);
          prevDirtyW = 440;
          prevDirtyH = 440;
          isIdle = true;
        }
        return;
      }
      isIdle = false;
      prevMx = mx;
      prevMy = my;

      // --- Dynamic trail parameters based on smoothSpeed ---
      // Dynamic trail length: slow move → long, fast swipe → short
      const MAX_TRAIL = 55;
      maxTrailLen = Math.round(MAX_TRAIL - Math.min(smoothSpeed, 20) * 1); // 55..35
      const decayRate = smoothSpeed < 1 ? 0.7 : Math.min(2.5, 0.7 + smoothSpeed * 0.07); // 0.7..2.5

      // Mouse Trail logic — add new points
      if (lastPlacedMouse.current.x === -1000) {
        lastPlacedMouse.current = { x: mx, y: my };
        handleResize();
      }

      const dx = mx - lastPlacedMouse.current.x;
      const dy = my - lastPlacedMouse.current.y;
      const dist = Math.sqrt(dx * dx + dy * dy);

      if (dist >= 1.5) {
        const steps = Math.max(1, Math.min(10, Math.floor(dist / 3)));
        for (let i = 1; i <= steps; i++) {
          points.current.unshift({
            x: lastPlacedMouse.current.x + dx * (i / steps),
            y: lastPlacedMouse.current.y + dy * (i / steps),
            r: isGazeActiveRef.current ? 4 : 3
          });
        }
        lastPlacedMouse.current = { x: mx, y: my };
      }

      // Trim to dynamic max length
      if (points.current.length > maxTrailLen) {
        points.current.splice(maxTrailLen);
      }

      // Dynamic decay: use accumulator for fractional rates
      decayAccumulator += decayRate;
      const popsThisFrame = Math.floor(decayAccumulator);
      decayAccumulator -= popsThisFrame;
      for (let i = 0; i < popsThisFrame; i++) {
        if (points.current.length > 0) points.current.pop();
      }

      // --- Compute dirty region (bounding box of cursor + all trail points) ---
      let minX = mx, maxX = mx, minY = my, maxY = my;
      for (let i = 0; i < points.current.length; i++) {
        const p = points.current[i];
        if (p.x < minX) minX = p.x;
        if (p.x > maxX) maxX = p.x;
        if (p.y < minY) minY = p.y;
        if (p.y > maxY) maxY = p.y;
      }
      // Pad for cursor radius + goo filter blur spread
      const pad = 20;
      const dirtyX = Math.floor(minX - pad);
      const dirtyY = Math.floor(minY - pad);
      const dirtyW = Math.ceil(maxX - minX + pad * 2);
      const dirtyH = Math.ceil(maxY - minY + pad * 2);

      // --- RENDER ---
      // Clear only previous frame's dirty region + current dirty region
      if (prevDirtyW > 0) {
        tctx.clearRect(prevDirtyX, prevDirtyY, prevDirtyW, prevDirtyH);
      }
      tctx.clearRect(dirtyX, dirtyY, dirtyW, dirtyH);
      octx.clearRect(0, 0, 400, 400);

      octx.save();
      octx.translate(200 - mx, 200 - my);
      octx.fillStyle = isGazeActiveRef.current ? GAZE_COLOR : brandHexRef.current;

      for (let i = 0; i < points.current.length; i++) {
        const p = points.current[i];
        const t = i / points.current.length;
        octx.globalAlpha = Math.max(0, 1 - t);
        octx.beginPath();
        octx.arc(p.x, p.y, p.r * (1 - t * 0.4), 0, Math.PI * 2);
        octx.fill();
      }

      octx.globalAlpha = 1;
      octx.beginPath();
      octx.arc(mx, my, isGazeActiveRef.current ? 6 : 4, 0, Math.PI * 2);
      octx.fill();

      if (isGazeActiveRef.current) {
          octx.strokeStyle = GAZE_COLOR;
          octx.lineWidth = 1;
          octx.beginPath();
          octx.arc(mx, my, 12 + Math.sin(Date.now() / 200) * 2, 0, Math.PI * 2);
          octx.stroke();
      }
      octx.restore();

      // Filter throttle: apply goo filter every 2 frames, reuse cache otherwise
      filterFrame++;
      if (filterFrame % 2 === 0 && filteredCtx && filteredCanvas) {
        filteredCtx.clearRect(0, 0, 400, 400);
        filteredCtx.save();
        filteredCtx.filter = 'url(#goo-cursor)';
        filteredCtx.drawImage(offscreenCanvas, 0, 0, 400, 400);
        filteredCtx.restore();
        prevFilterMx = mx;
        prevFilterMy = my;
      }

      if (filteredCanvas) {
        tctx.drawImage(filteredCanvas, prevFilterMx - 200, prevFilterMy - 200, 400, 400);
      }

      // Store dirty region for next frame
      prevDirtyX = dirtyX;
      prevDirtyY = dirtyY;
      prevDirtyW = dirtyW;
      prevDirtyH = dirtyH;
    };

    render();

    return () => {
      window.removeEventListener('resize', handleResize);
      window.removeEventListener('mousemove', handleMouseMove);
      cancelAnimationFrame(rafId);
    };
  }, []);

  return (
    <>
      <svg style={{ position: 'absolute', width: 0, height: 0, pointerEvents: 'none' }}>
        <defs>
          <filter id="goo-cursor">
            <feGaussianBlur in="SourceGraphic" stdDeviation="2" result="blur" />
            <feColorMatrix in="blur" mode="matrix" values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 18 -7" result="goo" />
          </filter>
        </defs>
      </svg>
      <canvas
        ref={trailCanvasRef}
        className="fixed inset-0 pointer-events-none z-[9999] w-full h-full"
      />
    </>
  );
}
