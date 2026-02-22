import { useRef, useEffect, useCallback } from 'react';
import { getDpr } from '../lib/canvasUtils';
import { useTheme } from '../hooks/useTheme';
import type { FitnessScores, FitnessLogEntry } from '../types';

/** Convert hex color to rgba string */
function hexRgba(hex: string, alpha: number): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return `rgba(${r},${g},${b},${alpha})`;
}

export const AXIS_COLORS: Record<string, string> = {
  cognitive: '#06b6d4',
  behavioral: '#22c55e',
  safety: '#ef4444',
  autonomy: '#a855f7',
  meta_learning: '#f59e0b',
};

export const AXIS_LABELS: Record<string, string> = {
  cognitive: 'Cognitive',
  behavioral: 'Behavioral',
  safety: 'Safety',
  autonomy: 'Autonomy',
  meta_learning: 'Meta-Learn',
};

export function FitnessChart({ timeline }: { timeline: FitnessLogEntry[] }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const mouseRef = useRef({ x: -1, y: -1 });
  const { colors } = useTheme();
  const colorsRef = useRef(colors);
  colorsRef.current = colors;
  const timelineRef = useRef(timeline);
  useEffect(() => { timelineRef.current = timeline; }, [timeline]);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const data = timelineRef.current;

    const dpr = getDpr();
    const w = canvas.width / dpr;
    const h = canvas.height / dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const textColor = colorsRef.current.canvasText;

    if (data.length < 2) {
      ctx.fillStyle = hexRgba(textColor, 0.2);
      ctx.font = '11px monospace';
      ctx.textAlign = 'center';
      const msg = data.length === 1
        ? 'One data point collected, need at least 2 to plot...'
        : 'Awaiting fitness data...';
      ctx.fillText(msg, w / 2, h / 2);
      return;
    }

    const pad = { top: 20, right: 16, bottom: 28, left: 40 };
    const cw = w - pad.left - pad.right;
    const ch = h - pad.top - pad.bottom;

    // Y-axis grid lines
    ctx.strokeStyle = hexRgba(textColor, 0.06);
    ctx.lineWidth = 1;
    for (let i = 0; i <= 5; i++) {
      const v = i / 5;
      const y = pad.top + ch * (1 - v);
      ctx.beginPath();
      ctx.moveTo(pad.left, y);
      ctx.lineTo(pad.left + cw, y);
      ctx.stroke();

      ctx.fillStyle = hexRgba(textColor, 0.3);
      ctx.font = '9px monospace';
      ctx.textAlign = 'right';
      ctx.fillText(v.toFixed(1), pad.left - 4, y + 3);
    }

    // Draw axis lines
    const axes: (keyof FitnessScores)[] = ['cognitive', 'behavioral', 'safety', 'autonomy', 'meta_learning'];
    const allLines: { key: string; color: string; pts: number[] }[] = axes.map(a => ({
      key: a,
      color: AXIS_COLORS[a],
      pts: data.map(e => e.scores[a]),
    }));
    allLines.push({
      key: 'fitness',
      color: textColor,
      pts: data.map(e => e.fitness),
    });

    for (const line of allLines) {
      ctx.strokeStyle = line.key === 'fitness' ? line.color : `${line.color}80`;
      ctx.lineWidth = line.key === 'fitness' ? 2 : 1;
      ctx.beginPath();
      for (let i = 0; i < line.pts.length; i++) {
        const x = pad.left + (i / (line.pts.length - 1)) * cw;
        const y = pad.top + ch * (1 - Math.min(1, Math.max(0, line.pts[i])));
        if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
      }
      ctx.stroke();
    }

    // X-axis labels (first and last timestamp)
    ctx.fillStyle = hexRgba(textColor, 0.3);
    ctx.font = '8px monospace';
    ctx.textAlign = 'left';
    const firstTs = new Date(data[0].timestamp).toLocaleTimeString();
    ctx.fillText(firstTs, pad.left, h - 4);
    ctx.textAlign = 'right';
    const lastTs = new Date(data[data.length - 1].timestamp).toLocaleTimeString();
    ctx.fillText(lastTs, w - pad.right, h - 4);

    // Hover crosshair
    const mx = mouseRef.current.x;
    if (mx >= pad.left && mx <= pad.left + cw) {
      const idx = Math.round(((mx - pad.left) / cw) * (data.length - 1));
      if (idx >= 0 && idx < data.length) {
        const entry = data[idx];
        const x = pad.left + (idx / (data.length - 1)) * cw;

        // Vertical line
        ctx.strokeStyle = hexRgba(textColor, 0.15);
        ctx.lineWidth = 1;
        ctx.setLineDash([4, 4]);
        ctx.beginPath();
        ctx.moveTo(x, pad.top);
        ctx.lineTo(x, pad.top + ch);
        ctx.stroke();
        ctx.setLineDash([]);

        // H-11: Tooltip with right-edge overflow prevention
        const tooltipW = 110;
        const tooltipH = 80;
        const tooltipY = pad.top + 4;
        const tooltipX = (x + 6 + tooltipW > w - pad.right) ? x - tooltipW - 6 : x + 6;
        ctx.fillStyle = hexRgba(colorsRef.current.canvasBg, 0.9);
        ctx.fillRect(tooltipX, tooltipY, tooltipW, tooltipH);
        ctx.fillStyle = textColor;
        ctx.font = 'bold 9px monospace';
        ctx.textAlign = 'left';
        ctx.fillText(`F: ${entry.fitness.toFixed(4)}`, tooltipX + 4, tooltipY + 12);

        let ty = tooltipY + 24;
        for (const a of axes) {
          ctx.fillStyle = AXIS_COLORS[a];
          ctx.fillText(`${AXIS_LABELS[a]}: ${entry.scores[a].toFixed(3)}`, tooltipX + 4, ty);
          ty += 11;
        }
      }
    }
  }, []);

  // Resize observer
  useEffect(() => {
    const container = containerRef.current;
    const canvas = canvasRef.current;
    if (!container || !canvas) return;

    const handleResize = () => {
      const dpr = getDpr();
      const w = container.clientWidth;
      const h = container.clientHeight;
      canvas.width = w * dpr;
      canvas.height = h * dpr;
      canvas.style.width = `${w}px`;
      canvas.style.height = `${h}px`;
      draw();
    };

    const ro = new ResizeObserver(handleResize);
    ro.observe(container);
    handleResize();

    return () => ro.disconnect();
  }, [draw]);

  // Redraw when timeline data changes
  useEffect(() => { draw(); }, [timeline, draw]);

  const rafRef = useRef(0);
  const handleMouseMove = useCallback((e: React.MouseEvent) => {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    mouseRef.current = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(draw);
  }, [draw]);

  const handleMouseLeave = useCallback(() => {
    mouseRef.current = { x: -1, y: -1 };
    draw();
  }, [draw]);

  return (
    <div ref={containerRef} className="w-full h-full relative">
      <canvas
        ref={canvasRef}
        className="w-full h-full"
        role="img"
        aria-label="Fitness timeline chart showing 5-axis scores over time"
        onMouseMove={handleMouseMove}
        onMouseLeave={handleMouseLeave}
      />
      {/* Legend */}
      <div className="absolute top-2 right-2 flex flex-wrap gap-x-3 gap-y-1">
        {Object.entries(AXIS_LABELS).map(([key, label]) => (
          <div key={key} className="flex items-center gap-1">
            <div className="w-2 h-[2px]" style={{ backgroundColor: AXIS_COLORS[key] }} />
            <span className="text-[8px] font-mono text-content-tertiary">{label}</span>
          </div>
        ))}
        <div className="flex items-center gap-1">
          <div className="w-2 h-[2px] bg-white" />
          <span className="text-[8px] font-mono text-content-tertiary">Fitness</span>
        </div>
      </div>
    </div>
  );
}
