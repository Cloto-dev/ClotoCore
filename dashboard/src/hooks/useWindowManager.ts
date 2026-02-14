import { useState, useCallback } from 'react';

export interface WindowInstance {
  id: string;
  type: string;
  title: string;
  x: number;
  y: number;
  zIndex: number;
}

// L-10: Maximum z-index before reset to prevent infinite growth
const MAX_Z_INDEX = 10000;

export function useWindowManager(initialZIndex = 100) {
  const [windows, setWindows] = useState<WindowInstance[]>([]);
  const [nextZ, setNextZ] = useState(initialZIndex);

  const getNextZ = useCallback(() => {
    if (nextZ >= MAX_Z_INDEX) {
      // Reset all window z-indices to base values
      setWindows(prev => prev.map((w, i) => ({ ...w, zIndex: initialZIndex + i })));
      setNextZ(initialZIndex + windows.length);
      return initialZIndex + windows.length;
    }
    return nextZ;
  }, [nextZ, initialZIndex, windows.length]);

  const openWindow = useCallback((item: { id: string, label: string }, x: number, y: number) => {
    const id = `${item.id}-${Date.now()}`;
    const z = getNextZ();
    setWindows(prev => [...prev, {
      id,
      type: item.id,
      title: item.label,
      x: Math.max(0, x - 400),
      y: Math.max(0, y - 20),
      zIndex: z
    }]);
    setNextZ(z + 1);
  }, [getNextZ]);

  const closeWindow = useCallback((id: string) => {
    setWindows(prev => prev.filter(w => w.id !== id));
  }, []);

  const focusWindow = useCallback((id: string) => {
    const z = getNextZ();
    setWindows(prev => prev.map(w => w.id === id ? { ...w, zIndex: z } : w));
    setNextZ(z + 1);
  }, [getNextZ]);

  return { windows, openWindow, closeWindow, focusWindow };
}
