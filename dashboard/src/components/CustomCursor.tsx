import { useEffect, useRef } from 'react';

interface Point { x: number; y: number; r: number }

export function CustomCursor() {
  const trailCanvasRef = useRef<HTMLCanvasElement>(null);
  const points = useRef<Point[]>([]);
  const lastPlacedMouse = useRef({ x: -1000, y: -1000 });
  const localMouse = useRef({ x: -1000, y: -1000 });
  const gazeTarget = useRef({ x: -1000, y: -1000 });
  const isGazeActiveRef = useRef(false);
  const gazeTimeoutRef = useRef<number>(0);

  // Listen for gaze events from GazeTracker via browser CustomEvent (no backend roundtrip)
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      if (detail && typeof detail.x === 'number') {
        gazeTarget.current = { x: detail.x, y: detail.y };
        isGazeActiveRef.current = true;
        // Auto-reset to mouse mode if no gaze data arrives within 500ms
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
    // Listen for mouse moves as fallback
    const handleMouseMove = (e: MouseEvent) => {
      // Only update localMouse directly if gaze is not dominant or user is actively moving mouse
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

    const initCanvas = () => {
      const canvas = trailCanvasRef.current;
      if (!canvas) return false;
      tctx = canvas.getContext('2d');
      offscreenCanvas = document.createElement('canvas');
      octx = offscreenCanvas.getContext('2d');
      if (!tctx || !octx) return false;
      return true;
    };

    const handleResize = () => {
      const canvas = trailCanvasRef.current;
      if (!canvas || !tctx || !octx || !offscreenCanvas) return;
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      const width = window.innerWidth;
      const height = window.innerHeight;
      canvas.width = width * dpr;
      canvas.height = height * dpr;
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;
      offscreenCanvas.width = 400 * dpr;
      offscreenCanvas.height = 400 * dpr;
      octx.setTransform(dpr, 0, 0, dpr, 0, 0);
      tctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };

    window.addEventListener('resize', handleResize);

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

      // Mouse Trail logic
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

      // Decay
      if (points.current.length > 50) points.current.splice(50);
      for (let i = 0; i < 2; i++) if (points.current.length > 0) points.current.pop();

      // --- RENDER ---
      tctx.clearRect(0, 0, window.innerWidth, window.innerHeight);
      octx.clearRect(0, 0, 400, 400);

      octx.save();
      octx.translate(200 - mx, 200 - my);
      octx.fillStyle = isGazeActiveRef.current ? '#ec4899' : '#2e4de6'; 
      
      for (let i = 0; i < points.current.length; i++) {
        const p = points.current[i];
        octx.globalAlpha = Math.max(0, 1 - i / points.current.length);
        octx.beginPath();
        octx.arc(p.x, p.y, p.r * (1 - (i / points.current.length) * 0.4), 0, Math.PI * 2);
        octx.fill();
      }
      
      octx.globalAlpha = 1;
      octx.beginPath();
      octx.arc(mx, my, isGazeActiveRef.current ? 6 : 4, 0, Math.PI * 2);
      octx.fill();

      if (isGazeActiveRef.current) {
          octx.strokeStyle = '#ec4899';
          octx.lineWidth = 1;
          octx.beginPath();
          octx.arc(mx, my, 12 + Math.sin(Date.now() / 200) * 2, 0, Math.PI * 2);
          octx.stroke();
      }
      octx.restore();

      tctx.save();
      tctx.filter = 'url(#goo-cursor)';
      tctx.drawImage(offscreenCanvas, mx - 200, my - 200, 400, 400);
      tctx.restore();
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
