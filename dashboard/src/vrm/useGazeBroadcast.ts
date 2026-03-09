import { useEffect, useRef } from 'react';

/**
 * Broadcasts normalized mouse position to the VRM viewer window via BroadcastChannel.
 * Only active when hasVrm is true.
 */
export function useGazeBroadcast(hasVrm: boolean) {
  const channelRef = useRef<BroadcastChannel | null>(null);

  useEffect(() => {
    if (!hasVrm) return;

    let channel: BroadcastChannel;
    try {
      channel = new BroadcastChannel('cloto-vrm-gaze');
    } catch {
      return; // BroadcastChannel not supported
    }
    channelRef.current = channel;

    const handleMouseMove = (e: MouseEvent) => {
      // Normalize to [-1, 1] based on window size
      const nx = (e.clientX / window.innerWidth) * 2 - 1;
      const ny = -((e.clientY / window.innerHeight) * 2 - 1);
      channel.postMessage({ nx, ny });
    };

    // Throttle to ~30fps to avoid flooding the channel
    let lastSend = 0;
    const throttledMove = (e: MouseEvent) => {
      const now = performance.now();
      if (now - lastSend < 33) return;
      lastSend = now;
      handleMouseMove(e);
    };

    window.addEventListener('mousemove', throttledMove);

    return () => {
      window.removeEventListener('mousemove', throttledMove);
      channel.close();
      channelRef.current = null;
    };
  }, [hasVrm]);
}
