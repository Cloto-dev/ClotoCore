import { useEffect, useRef } from 'react';
import { useTheme } from '../hooks/useTheme';
import { getDpr } from '../lib/canvasUtils';

export function FadingGrid() {
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

      const gridSize = 40;

      // Fade: full opacity at top 40%, transparent at bottom
      const fadeStart = height * 0.4;

      // Draw horizontal lines with vertical fade
      for (let y = 0; y <= height + gridSize; y += gridSize) {
        const alpha = y < fadeStart ? 0.4 : 0.4 * Math.max(0, 1 - (y - fadeStart) / (height - fadeStart));
        gctx.globalAlpha = alpha;
        gctx.beginPath();
        gctx.moveTo(0, y);
        gctx.lineTo(width, y);
        gctx.stroke();
      }
      // Draw vertical lines with vertical fade (use mask approach)
      for (let x = 0; x <= width + gridSize; x += gridSize) {
        // Draw in segments to apply fade
        for (let y = 0; y < height; y += gridSize) {
          const midY = y + gridSize / 2;
          const alpha = midY < fadeStart ? 0.4 : 0.4 * Math.max(0, 1 - (midY - fadeStart) / (height - fadeStart));
          gctx.globalAlpha = alpha;
          gctx.beginPath();
          gctx.moveTo(x, y);
          gctx.lineTo(x, Math.min(y + gridSize, height));
          gctx.stroke();
        }
      }
    };

    window.addEventListener('resize', draw);
    draw();

    return () => {
      window.removeEventListener('resize', draw);
    };
  }, []);

  return <canvas ref={gridCanvasRef} className="absolute inset-0 pointer-events-none z-0 w-full h-full" />;
}
