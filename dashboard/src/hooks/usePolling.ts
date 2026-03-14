import { useEffect, useRef } from 'react';

/**
 * Run a callback on a fixed interval with automatic cleanup.
 * Calls `fn` immediately on mount, then every `intervalMs` milliseconds.
 * Set `enabled` to false to pause polling without destroying the hook.
 */
export function usePolling(
  fn: () => void | Promise<void>,
  intervalMs: number,
  enabled = true,
) {
  const fnRef = useRef(fn);
  fnRef.current = fn;

  useEffect(() => {
    if (!enabled) return;
    fnRef.current();
    const id = setInterval(() => fnRef.current(), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs, enabled]);
}
