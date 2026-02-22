import { useEffect, useRef } from 'react';
import { useTheme } from '../hooks/useTheme';
import { getDpr } from '../lib/canvasUtils';

export function InteractiveGrid() {
  const gridCanvasRef = useRef<HTMLCanvasElement>(null);
  const { colors } = useTheme();
  const colorsRef = useRef(colors);
  colorsRef.current = colors;

  useEffect(() => {
    const gridCanvas = gridCanvasRef.current;
    if (!gridCanvas) return;

    const gctx = gridCanvas.getContext('2d', { alpha: false });
    if (!gctx) return;

    const draw = () => {
      const dpr = getDpr();
      const width = window.innerWidth;
      const height = window.innerHeight;

      gridCanvas.width = width * dpr;
      gridCanvas.height = height * dpr;
      gridCanvas.style.width = `${width}px`;
      gridCanvas.style.height = `${height}px`;

      gctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      // Background color
      gctx.fillStyle = colorsRef.current.canvasBg;
      gctx.fillRect(0, 0, width, height);

      // Grid lines
      gctx.strokeStyle = colorsRef.current.canvasGrid;
      gctx.lineWidth = 1;
      gctx.globalAlpha = 0.4;

      const gridSize = 40;

      // Draw horizontal lines
      for (let y = 0; y <= height + gridSize; y += gridSize) {
        gctx.beginPath();
        gctx.moveTo(0, y);
        gctx.lineTo(width, y);
        gctx.stroke();
      }
      // Draw vertical lines
      for (let x = 0; x <= width + gridSize; x += gridSize) {
        gctx.beginPath();
        gctx.moveTo(x, 0);
        gctx.lineTo(x, height);
        gctx.stroke();
      }
    };

    window.addEventListener('resize', draw);
    draw();

    return () => {
      window.removeEventListener('resize', draw);
    };
  }, [colors]);

  return (
    <canvas 
      ref={gridCanvasRef} 
      className="absolute inset-0 pointer-events-none z-0 w-full h-full" 
    />
  );
}