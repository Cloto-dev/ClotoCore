import { useState, useEffect, useRef, useCallback } from 'react';

interface UseTypewriterOptions {
  text: string;
  speed?: number;       // ms per character (default: 5)
  enabled?: boolean;    // default: true
  onComplete?: () => void;
}

interface UseTypewriterResult {
  displayText: string;
  isAnimating: boolean;
  skip: () => void;
  progress: number;     // 0..1
}

export function useTypewriter({
  text,
  speed = 5,
  enabled = true,
  onComplete,
}: UseTypewriterOptions): UseTypewriterResult {
  const [displayText, setDisplayText] = useState('');
  const [isAnimating, setIsAnimating] = useState(false);

  const rafRef = useRef<number>(0);
  const startTimeRef = useRef(0);
  const skippedRef = useRef(false);
  const onCompleteRef = useRef(onComplete);
  onCompleteRef.current = onComplete;

  const skip = useCallback(() => {
    if (!isAnimating) return;
    skippedRef.current = true;
    cancelAnimationFrame(rafRef.current);
    setDisplayText(text);
    setIsAnimating(false);
    onCompleteRef.current?.();
  }, [isAnimating, text]);

  useEffect(() => {
    if (!enabled || !text) {
      setDisplayText('');
      setIsAnimating(false);
      return;
    }

    skippedRef.current = false;
    setIsAnimating(true);
    startTimeRef.current = 0;

    const BATCH_INTERVAL = 50; // ms between React state updates
    let lastUpdateTime = 0;

    const tick = (timestamp: number) => {
      if (skippedRef.current) return;

      if (!startTimeRef.current) {
        startTimeRef.current = timestamp;
        lastUpdateTime = timestamp;
      }

      const elapsed = timestamp - startTimeRef.current;
      const charIndex = Math.min(Math.floor(elapsed / speed), text.length);

      // Batch updates to ~20/sec for performance
      if (timestamp - lastUpdateTime >= BATCH_INTERVAL || charIndex >= text.length) {
        setDisplayText(text.slice(0, charIndex));
        lastUpdateTime = timestamp;
      }

      if (charIndex >= text.length) {
        setIsAnimating(false);
        onCompleteRef.current?.();
        return;
      }

      rafRef.current = requestAnimationFrame(tick);
    };

    rafRef.current = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(rafRef.current);
    };
  }, [text, speed, enabled]);

  const progress = text.length > 0 ? displayText.length / text.length : 0;

  return { displayText, isAnimating, skip, progress };
}
