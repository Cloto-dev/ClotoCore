import { useState, useRef, useEffect, useCallback, memo } from 'react';
import { getDpr } from '../lib/canvasUtils';
import { useApiKey } from '../contexts/ApiKeyContext';
import { Link } from 'react-router-dom';
import {
  ArrowLeft, TrendingUp, TrendingDown, Shield, Brain, Zap,
  Settings, Activity, Eye, Minus, Lock,
} from 'lucide-react';
import { useEvolution } from '../hooks/useEvolution';
import { api } from '../services/api';
import type { FitnessScores, FitnessLogEntry, EvolutionParams } from '../types';

const AXIS_COLORS: Record<string, string> = {
  cognitive: '#06b6d4',
  behavioral: '#22c55e',
  safety: '#ef4444',
  autonomy: '#a855f7',
  meta_learning: '#f59e0b',
};

const AXIS_ICONS: Record<string, typeof Brain> = {
  cognitive: Brain,
  behavioral: Activity,
  safety: Shield,
  autonomy: Zap,
  meta_learning: Eye,
};

const AXIS_LABELS: Record<string, string> = {
  cognitive: 'Cognitive',
  behavioral: 'Behavioral',
  safety: 'Safety',
  autonomy: 'Autonomy',
  meta_learning: 'Meta-Learn',
};

type Tab = 'generations' | 'events' | 'rollbacks' | 'params';

// --- Scoreboard Panel ---
function ScoreboardPanel({ status }: { status: ReturnType<typeof useEvolution>['status'] }) {
  if (!status) {
    return (
      <div className="flex items-center justify-center h-full text-content-tertiary text-xs font-mono">
        NO EVOLUTION DATA
      </div>
    );
  }

  const scores = status.scores;
  const axes = Object.keys(AXIS_COLORS) as (keyof FitnessScores)[];
  const trendUp = status.trend === 'improving';
  const trendDown = status.trend === 'declining';

  return (
    <div className="flex flex-col gap-3 p-4">
      {/* Header: Generation + Autonomy */}
      <div className="flex items-center justify-between">
        <div>
          <div className="text-[10px] font-mono text-content-tertiary uppercase tracking-widest">Generation</div>
          <div className="text-3xl font-black tabular-nums">{status.current_generation}</div>
        </div>
        <div className="text-right">
          <div className="text-[10px] font-mono text-content-tertiary uppercase tracking-widest">Autonomy</div>
          <div className="text-sm font-bold text-purple-400">{status.autonomy_level}</div>
        </div>
      </div>

      {/* Fitness + Trend */}
      <div className="flex items-center gap-2">
        <div className="text-[10px] font-mono text-content-tertiary uppercase tracking-widest">Fitness</div>
        <div className="text-lg font-black tabular-nums">{status.fitness.toFixed(4)}</div>
        {trendUp && <TrendingUp size={14} className="text-green-400" />}
        {trendDown && <TrendingDown size={14} className="text-red-400" />}
        {!trendUp && !trendDown && <Minus size={14} className="text-content-tertiary" />}
      </div>

      {/* 5-Axis Scores */}
      <div className="flex flex-col gap-2">
        {axes.map(axis => {
          const Icon = AXIS_ICONS[axis];
          const val = scores[axis];
          return (
            <div key={axis} className="flex items-center gap-2">
              <Icon size={12} style={{ color: AXIS_COLORS[axis] }} />
              <span className="text-[10px] font-mono text-content-secondary w-20">{AXIS_LABELS[axis]}</span>
              <div className="flex-1 h-2 rounded-full bg-white/5 overflow-hidden">
                <div
                  className="h-full rounded-full transition-all duration-500"
                  style={{ width: `${val * 100}%`, backgroundColor: AXIS_COLORS[axis] }}
                />
              </div>
              <span className="text-[10px] font-mono tabular-nums w-10 text-right">{val.toFixed(2)}</span>
            </div>
          );
        })}
      </div>

      {/* Interaction Count */}
      <div className="text-[10px] font-mono text-content-tertiary">
        {status.interaction_count} interactions
      </div>

      {/* Grace Period Warning */}
      {status.grace_period?.active && (
        <div className="p-2 rounded-lg bg-amber-500/10 border border-amber-500/30 text-[10px] font-mono text-amber-400">
          GRACE PERIOD: {status.grace_period.affected_axis} axis regression detected.
          {Math.max(0, status.grace_period.grace_interactions - (status.interaction_count - status.grace_period.interactions_at_start))} interactions remaining.
        </div>
      )}
    </div>
  );
}

// --- Canvas Timeline Chart ---
function FitnessChart({ timeline }: { timeline: FitnessLogEntry[] }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const mouseRef = useRef({ x: -1, y: -1 });
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

    if (data.length < 2) {
      ctx.fillStyle = 'rgba(255,255,255,0.2)';
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
    ctx.strokeStyle = 'rgba(255,255,255,0.06)';
    ctx.lineWidth = 1;
    for (let i = 0; i <= 5; i++) {
      const v = i / 5;
      const y = pad.top + ch * (1 - v);
      ctx.beginPath();
      ctx.moveTo(pad.left, y);
      ctx.lineTo(pad.left + cw, y);
      ctx.stroke();

      ctx.fillStyle = 'rgba(255,255,255,0.3)';
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
      color: '#ffffff',
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
    ctx.fillStyle = 'rgba(255,255,255,0.3)';
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
        ctx.strokeStyle = 'rgba(255,255,255,0.15)';
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
        ctx.fillStyle = 'rgba(0,0,0,0.8)';
        ctx.fillRect(tooltipX, tooltipY, tooltipW, tooltipH);
        ctx.fillStyle = '#ffffff';
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

// --- Tabbed Bottom Panel ---
function BottomPanel({ data }: { data: ReturnType<typeof useEvolution> }) {
  const [tab, setTab] = useState<Tab>('generations');
  const [showParamModal, setShowParamModal] = useState(false);
  const [paramRefreshKey, setParamRefreshKey] = useState(0);

  const tabs: { id: Tab; label: string }[] = [
    { id: 'generations', label: 'GENERATIONS' },
    { id: 'events', label: 'EVENTS' },
    { id: 'rollbacks', label: 'ROLLBACKS' },
    { id: 'params', label: 'PARAMS' },
  ];

  return (
    <div className="flex flex-col h-full">
      {/* Tab Bar */}
      <div className="flex gap-1 p-2 border-b border-white/5" role="tablist">
        {tabs.map(t => (
          <button
            key={t.id}
            id={`evo-tab-${t.id}`}
            role="tab"
            aria-selected={tab === t.id}
            aria-controls={`evo-panel-${t.id}`}
            onClick={() => setTab(t.id)}
            className={`px-3 py-1 rounded text-[9px] font-black uppercase tracking-wider transition-all ${
              tab === t.id
                ? 'bg-white/10 text-content-primary'
                : 'text-content-tertiary hover:text-content-secondary'
            }`}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* Tab Content */}
      <div className="flex-1 overflow-auto p-3" role="tabpanel" id={`evo-panel-${tab}`} aria-labelledby={`evo-tab-${tab}`}>
        {tab === 'generations' && <GenerationsTab generations={data.generations} />}
        {tab === 'events' && <EventsTab events={data.events} />}
        {tab === 'rollbacks' && <RollbacksTab rollbacks={data.rollbacks} />}
        {tab === 'params' && <ParamsTab onEdit={() => setShowParamModal(true)} refreshKey={paramRefreshKey} />}
      </div>

      {showParamModal && (
        <ParamEditModal
          onClose={() => setShowParamModal(false)}
          onSuccess={() => { setShowParamModal(false); setParamRefreshKey(k => k + 1); data.refresh(); }}
        />
      )}
    </div>
  );
}

function GenerationsTab({ generations }: { generations: ReturnType<typeof useEvolution>['generations'] }) {
  if (generations.length === 0) {
    return <div className="text-content-tertiary text-xs font-mono">No generation records yet.</div>;
  }
  return (
    <table className="w-full text-[10px] font-mono">
      <thead>
        <tr className="text-content-tertiary text-left">
          <th className="pr-3 pb-1">GEN</th>
          <th className="pr-3 pb-1">TRIGGER</th>
          <th className="pr-3 pb-1">FITNESS</th>
          <th className="pr-3 pb-1">DELTA</th>
          <th className="pb-1">TIME</th>
        </tr>
      </thead>
      <tbody>
        {generations.map(g => (
          <tr key={g.generation} className="border-t border-white/5">
            <td className="pr-3 py-1 font-bold">{g.generation}</td>
            <td className="pr-3 py-1">
              <span className={`px-1.5 py-0.5 rounded text-[8px] font-bold ${
                g.trigger === 'Evolution' ? 'bg-green-500/20 text-green-400' :
                g.trigger === 'Regression' ? 'bg-red-500/20 text-red-400' :
                g.trigger === 'SafetyBreach' ? 'bg-red-600/20 text-red-500' :
                'bg-blue-500/20 text-blue-400'
              }`}>
                {g.trigger}
              </span>
            </td>
            <td className="pr-3 py-1 tabular-nums">{g.fitness.toFixed(4)}</td>
            <td className={`pr-3 py-1 tabular-nums ${g.fitness_delta >= 0 ? 'text-green-400' : 'text-red-400'}`}>
              {g.fitness_delta >= 0 ? '+' : ''}{g.fitness_delta.toFixed(4)}
            </td>
            <td className="py-1 text-content-tertiary">{new Date(g.timestamp).toLocaleString()}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function EventsTab({ events }: { events: ReturnType<typeof useEvolution>['events'] }) {
  if (events.length === 0) {
    return <div className="text-content-tertiary text-xs font-mono">Waiting for evolution events...</div>;
  }
  return (
    <div className="flex flex-col gap-1">
      {events.map((evt, i) => (
        <div key={`${evt.timestamp}-${evt.type}-${i}`} className="flex items-start gap-2 text-[10px] font-mono py-1 border-b border-white/5">
          <span className="text-content-tertiary w-16 shrink-0">
            {new Date(evt.timestamp).toLocaleTimeString()}
          </span>
          <span className="font-bold text-cyan-400">{evt.type}</span>
          <span className="text-content-secondary truncate">{JSON.stringify(evt.data).slice(0, 120)}</span>
        </div>
      ))}
    </div>
  );
}

function RollbacksTab({ rollbacks }: { rollbacks: ReturnType<typeof useEvolution>['rollbacks'] }) {
  if (rollbacks.length === 0) {
    return <div className="text-content-tertiary text-xs font-mono">No rollback history.</div>;
  }
  return (
    <table className="w-full text-[10px] font-mono">
      <thead>
        <tr className="text-content-tertiary text-left">
          <th className="pr-3 pb-1">FROM</th>
          <th className="pr-3 pb-1">TO</th>
          <th className="pr-3 pb-1">REASON</th>
          <th className="pr-3 pb-1">COUNT</th>
          <th className="pb-1">TIME</th>
        </tr>
      </thead>
      <tbody>
        {rollbacks.map((r, i) => (
          <tr key={`${r.from_generation}-${r.timestamp}-${i}`} className="border-t border-white/5">
            <td className="pr-3 py-1 text-red-400 font-bold">G{r.from_generation}</td>
            <td className="pr-3 py-1 text-green-400 font-bold">G{r.to_generation}</td>
            <td className="pr-3 py-1">{r.reason}</td>
            <td className="pr-3 py-1 tabular-nums">{r.rollback_count_to_target}</td>
            <td className="py-1 text-content-tertiary">{new Date(r.timestamp).toLocaleString()}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function ParamsTab({ onEdit, refreshKey }: { onEdit: () => void; refreshKey: number }) {
  const [params, setParams] = useState<EvolutionParams | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api.getEvolutionParams().then(p => { if (!cancelled) setParams(p); }).catch(e => { if (!cancelled) setError(e.message); });
    return () => { cancelled = true; };
  }, [refreshKey]);

  if (error) {
    return <div className="text-red-400 text-xs font-mono">Failed to load params: {error}</div>;
  }
  if (!params) {
    return <div className="text-content-tertiary text-xs font-mono">Loading parameters...</div>;
  }

  const entries = [
    ['alpha (growth threshold)', params.alpha],
    ['beta (regression threshold)', params.beta],
    ['theta_min (minimum threshold)', params.theta_min],
    ['gamma (rebalance threshold)', params.gamma],
    ['min_interactions (debounce)', params.min_interactions],
  ];

  const weightEntries = Object.entries(params.weights).map(([k, v]) => [
    `weight.${k}`, v,
  ]);

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-1">
        {entries.map(([label, val]) => (
          <div key={String(label)} className="flex justify-between text-[10px] font-mono py-0.5">
            <span className="text-content-secondary">{label}</span>
            <span className="tabular-nums">{String(val)}</span>
          </div>
        ))}
      </div>
      <div className="border-t border-white/5 pt-2">
        <div className="text-[9px] font-mono text-content-tertiary uppercase tracking-widest mb-1">Weights</div>
        {weightEntries.map(([label, val]) => (
          <div key={String(label)} className="flex justify-between text-[10px] font-mono py-0.5">
            <span className="text-content-secondary">{label}</span>
            <span className="tabular-nums">{String(val)}</span>
          </div>
        ))}
      </div>
      <button
        onClick={onEdit}
        className="flex items-center gap-1 px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] font-bold text-content-secondary hover:text-content-primary hover:bg-white/10 transition-all self-start"
      >
        <Settings size={12} /> EDIT PARAMS
      </button>
    </div>
  );
}

function ParamEditModal({ onClose, onSuccess }: { onClose: () => void; onSuccess: () => void }) {
  const { apiKey } = useApiKey();
  const [error, setError] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [params, setParams] = useState<EvolutionParams | null>(null);
  const [form, setForm] = useState({
    alpha: '', beta: '', theta_min: '', gamma: '', min_interactions: '',
  });

  useEffect(() => {
    let cancelled = false;
    api.getEvolutionParams().then(p => {
      if (cancelled) return;
      setParams(p);
      setForm({
        alpha: String(p.alpha),
        beta: String(p.beta),
        theta_min: String(p.theta_min),
        gamma: String(p.gamma),
        min_interactions: String(p.min_interactions),
      });
    }).catch(e => { if (!cancelled) setError(e.message); });
    return () => { cancelled = true; };
  }, []);

  const handleSave = async () => {
    if (!apiKey) return;
    setIsLoading(true);
    setError('');
    try {
      const alpha = parseFloat(form.alpha);
      const beta = parseFloat(form.beta);
      const theta_min = parseFloat(form.theta_min);
      const gamma = parseFloat(form.gamma);
      const min_interactions = parseInt(form.min_interactions);
      const values = [alpha, beta, theta_min, gamma, min_interactions];
      if (values.some(v => isNaN(v) || !isFinite(v))) {
        setError('All fields must be valid numbers');
        setIsLoading(false);
        return;
      }
      const update: EvolutionParams = { alpha, beta, theta_min, gamma, min_interactions, weights: params!.weights };
      await api.updateEvolutionParams(update, apiKey);
      onSuccess();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : 'Failed to update params');
    } finally {
      setIsLoading(false);
    }
  };

  if (!params) return null;

  const fields: { key: keyof typeof form; label: string }[] = [
    { key: 'alpha', label: 'Alpha (growth)' },
    { key: 'beta', label: 'Beta (regression)' },
    { key: 'theta_min', label: 'Theta Min' },
    { key: 'gamma', label: 'Gamma (rebalance)' },
    { key: 'min_interactions', label: 'Min Interactions' },
  ];

  return (
    <div
      className="absolute inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-label="Edit Evolution Params"
      onKeyDown={e => e.key === 'Escape' && onClose()}
    >
      <div className="bg-[#1a1a2e] rounded-2xl shadow-2xl p-6 w-80 space-y-3 border border-white/10">
        <div className="flex items-center gap-2">
          <Settings size={16} className="text-content-secondary" />
          <h3 className="text-sm font-bold text-content-primary">Edit Evolution Params</h3>
        </div>

        {fields.map(f => (
          <div key={f.key}>
            <label className="text-[9px] font-mono text-content-tertiary uppercase">{f.label}</label>
            <input
              type="number"
              step="any"
              value={form[f.key]}
              onChange={e => setForm(prev => ({ ...prev, [f.key]: e.target.value }))}
              className="w-full px-2 py-1.5 rounded-lg border border-white/10 bg-white/5 text-xs font-mono focus:outline-none focus:border-purple-400"
            />
          </div>
        ))}

        {!apiKey && (
          <p className="text-[10px] text-amber-400 font-mono pt-2 border-t border-white/10">
            API Key が未設定です。画面上部の 🔒 から設定してください。
          </p>
        )}

        {error && <p className="text-[10px] text-red-400 font-medium">{error}</p>}

        <div className="flex gap-2 pt-1">
          <button
            onClick={onClose}
            className="flex-1 py-2 rounded-xl border border-white/10 text-xs font-bold text-content-secondary hover:bg-white/5 transition-all"
            disabled={isLoading}
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={!apiKey || isLoading}
            className="flex-1 py-2 rounded-xl bg-purple-600 hover:bg-purple-700 text-white text-xs font-bold transition-all disabled:opacity-50"
          >
            {isLoading ? 'Saving...' : 'Save'}
          </button>
        </div>
      </div>
    </div>
  );
}

// --- Main Component ---
export const EvolutionCore = memo(function EvolutionCore({ isWindowMode = false }: { isWindowMode?: boolean }) {
  const evo = useEvolution();

  if (evo.loading) {
    return (
      <div className={`${isWindowMode ? 'h-full' : 'min-h-screen'} flex items-center justify-center bg-surface-base text-content-tertiary font-mono text-xs`}>
        LOADING EVOLUTION ENGINE...
      </div>
    );
  }

  if (evo.error && !evo.status) {
    return (
      <div className={`${isWindowMode ? 'h-full' : 'min-h-screen'} flex flex-col items-center justify-center bg-surface-base gap-4`}>
        <div className="text-content-tertiary font-mono text-xs">EVOLUTION ENGINE OFFLINE</div>
        <div className="text-[10px] text-red-400 font-mono max-w-md text-center">{evo.error}</div>
        <button
          onClick={evo.refresh}
          className="px-4 py-2 rounded-full bg-white/5 border border-white/10 text-xs font-bold text-content-secondary hover:text-content-primary transition-all"
        >
          RETRY
        </button>
        {!isWindowMode && (
          <Link to="/" className="flex items-center gap-2 px-4 py-2 rounded-full bg-white/5 border border-white/10 text-xs font-bold text-content-secondary hover:text-content-primary transition-all">
            <ArrowLeft size={14} /> BACK
          </Link>
        )}
      </div>
    );
  }

  return (
    <div className={`${isWindowMode ? 'h-full' : 'min-h-screen'} bg-surface-base text-content-primary flex flex-col relative`}>
      {/* Header */}
      {!isWindowMode && (
        <div className="flex items-center justify-between px-6 pt-6 pb-2">
          <Link to="/" className="flex items-center gap-2 px-4 py-2 rounded-full bg-white/5 backdrop-blur-sm border border-white/10 text-xs font-bold text-content-secondary hover:text-content-primary transition-all">
            <ArrowLeft size={14} /> BACK TO INTERFACE
          </Link>
          <div className="text-right">
            <h2 className="text-xl font-black tracking-tighter uppercase">Self-Evolution</h2>
            <p className="text-[10px] font-mono text-content-tertiary uppercase tracking-widest">Benchmark Engine</p>
          </div>
        </div>
      )}

      {/* 3-Panel Grid */}
      <div className={`flex-1 grid grid-cols-2 grid-rows-[1fr_1fr] gap-3 ${isWindowMode ? 'p-2' : 'p-4'}`}>
        {/* Panel 1: Scoreboard (top-left) */}
        <div className="bg-black/40 backdrop-blur-md border border-white/10 rounded-xl overflow-hidden">
          <ScoreboardPanel status={evo.status} />
        </div>

        {/* Panel 2: Fitness Chart (top-right) */}
        <div className="bg-black/40 backdrop-blur-md border border-white/10 rounded-xl overflow-hidden p-2">
          <div className="text-[9px] font-mono text-content-tertiary uppercase tracking-widest mb-1 px-2">Fitness Timeline</div>
          <div className="w-full" style={{ height: 'calc(100% - 20px)' }}>
            <FitnessChart timeline={evo.timeline} />
          </div>
        </div>

        {/* Panel 3: Tabbed Bottom (spans full width) */}
        <div className="col-span-2 bg-black/40 backdrop-blur-md border border-white/10 rounded-xl overflow-hidden relative">
          <BottomPanel data={evo} />
        </div>
      </div>
    </div>
  );
});
