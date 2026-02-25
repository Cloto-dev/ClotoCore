import { useState, useRef, useEffect } from 'react';

export function useLongPress(durationMs: number, onComplete: () => void) {
  const [progress, setProgress] = useState(0);
  const rafRef = useRef<number>(0);
  const startRef = useRef(0);

  const start = () => {
    startRef.current = Date.now();
    const tick = () => {
      const elapsed = Date.now() - startRef.current;
      const p = Math.min(elapsed / durationMs, 1);
      setProgress(p);
      if (p >= 1) {
        onComplete();
        setProgress(0);
        return;
      }
      rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);
  };

  const cancel = () => {
    cancelAnimationFrame(rafRef.current);
    setProgress(0);
  };

  useEffect(() => () => cancelAnimationFrame(rafRef.current), []);

  const handlers = {
    onMouseDown: start,
    onMouseUp: cancel,
    onMouseLeave: cancel,
    onTouchStart: start,
    onTouchEnd: cancel,
  };

  return { progress, handlers };
}
