import { useState, useRef, useEffect, useCallback } from 'react';

export function useDraggable(onDrop: (item: any, x: number, y: number) => void, onClick: (item: any) => void) {
  const [ghostPos, setGhostPos] = useState<{ x: number, y: number } | null>(null);
  const [isTracking, setIsTracking] = useState(false);
  
  const dragRef = useRef<{
    startX: number;
    startY: number;
    item: any;
    hasDetached: boolean;
  }>({ startX: 0, startY: 0, item: null, hasDetached: false });

  const handleMouseDown = useCallback((e: React.MouseEvent, item: any) => {
    if (item.disabled) return;
    if (e.button !== 0) return;
    
    dragRef.current = { 
      startX: e.clientX, 
      startY: e.clientY, 
      item, 
      hasDetached: false 
    };
    setIsTracking(true);
  }, []);

  useEffect(() => {
    if (!isTracking) return;

    const handleMouseMove = (e: MouseEvent) => {
      const dist = Math.hypot(e.clientX - dragRef.current.startX, e.clientY - dragRef.current.startY);
      if (dist > 30 && !dragRef.current.hasDetached) {
        dragRef.current.hasDetached = true;
      }

      if (dragRef.current.hasDetached) {
        setGhostPos({ x: e.clientX, y: e.clientY });
      }
    };

    const handleMouseUp = (e: MouseEvent) => {
      if (dragRef.current.hasDetached) {
        onDrop(dragRef.current.item, e.clientX, e.clientY);
      } else {
        const dist = Math.hypot(e.clientX - dragRef.current.startX, e.clientY - dragRef.current.startY);
        if (dist < 15) { // クリック許容範囲を少し広めに設定
          onClick(dragRef.current.item);
        }
      }
      
      setIsTracking(false);
      setGhostPos(null);
    };
    
    window.addEventListener('mousemove', handleMouseMove);
    window.addEventListener('mouseup', handleMouseUp);

    return () => {
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isTracking, onDrop, onClick]);

  return { ghostPos, handleMouseDown, dragItem: dragRef.current.item };
}