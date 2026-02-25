import { useState, useEffect, memo } from 'react';
import { Link } from 'react-router-dom';
import {
  ArrowLeft, TrendingUp, TrendingDown, Shield, Brain, Zap,
  Settings, Activity, Eye, Minus,
} from 'lucide-react';
import { useEvolution } from '../hooks/useEvolution';
import { api } from '../services/api';
import { FitnessChart, AXIS_COLORS, AXIS_LABELS } from './FitnessChart';
import { ParamEditModal } from './ParamEditModal';
import type { FitnessScores, EvolutionParams } from '../types';

const AXIS_ICONS: Record<string, typeof Brain> = {
  cognitive: Brain,
  behavioral: Activity,
  safety: Shield,
  autonomy: Zap,
  meta_learning: Eye,
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
              <div className="flex-1 h-2 rounded-full bg-surface-secondary/50 overflow-hidden">
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
      <div className="flex gap-1 p-2 border-b border-edge-subtle" role="tablist">
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
                ? 'bg-glass-subtle text-content-primary'
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
          <tr key={g.generation} className="border-t border-edge-subtle">
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
        <div key={`${evt.timestamp}-${evt.type}-${i}`} className="flex items-start gap-2 text-[10px] font-mono py-1 border-b border-edge-subtle">
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
          <tr key={`${r.from_generation}-${r.timestamp}-${i}`} className="border-t border-edge-subtle">
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
      <div className="border-t border-edge-subtle pt-2">
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
        className="flex items-center gap-1 px-3 py-1.5 rounded-lg bg-surface-secondary/50 border border-edge text-[10px] font-bold text-content-secondary hover:text-content-primary hover:bg-glass-subtle transition-all self-start"
      >
        <Settings size={12} /> EDIT PARAMS
      </button>
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
          className="px-4 py-2 rounded-full bg-surface-secondary/50 border border-edge text-xs font-bold text-content-secondary hover:text-content-primary transition-all"
        >
          RETRY
        </button>
        {!isWindowMode && (
          <Link to="/" className="flex items-center gap-2 px-4 py-2 rounded-full bg-surface-secondary/50 border border-edge text-xs font-bold text-content-secondary hover:text-content-primary transition-all">
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
          <Link to="/" className="flex items-center gap-2 px-4 py-2 rounded-full bg-surface-secondary/50 backdrop-blur-sm border border-edge text-xs font-bold text-content-secondary hover:text-content-primary transition-all">
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
        <div className="bg-glass backdrop-blur-md border border-edge rounded-xl overflow-hidden">
          <ScoreboardPanel status={evo.status} />
        </div>

        {/* Panel 2: Fitness Chart (top-right) */}
        <div className="bg-glass backdrop-blur-md border border-edge rounded-xl overflow-hidden p-2">
          <div className="text-[9px] font-mono text-content-tertiary uppercase tracking-widest mb-1 px-2">Fitness Timeline</div>
          <div className="w-full" style={{ height: 'calc(100% - 20px)' }}>
            <FitnessChart timeline={evo.timeline} />
          </div>
        </div>

        {/* Panel 3: Tabbed Bottom (spans full width) */}
        <div className="col-span-2 bg-glass backdrop-blur-md border border-edge rounded-xl overflow-hidden relative">
          <BottomPanel data={evo} />
        </div>
      </div>
    </div>
  );
});
