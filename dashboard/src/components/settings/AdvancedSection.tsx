import { useState, useEffect } from 'react';
import { AlertTriangle } from 'lucide-react';
import { SectionCard, Toggle } from './common';
import { useApi } from '../../hooks/useApi';

export function AdvancedSection() {
  const api = useApi();
  const [yoloEnabled, setYoloEnabled] = useState(false);
  const [loading, setLoading] = useState(true);
  const [maxCronGen, setMaxCronGen] = useState(2);

  useEffect(() => {
    api.fetchJson<{ enabled: boolean }>('/settings/yolo')
      .then(data => setYoloEnabled(data.enabled))
      .catch(() => {})
      .finally(() => setLoading(false));
    api.fetchJson<{ value: number }>('/settings/max-cron-generation')
      .then(data => setMaxCronGen(data.value))
      .catch(() => {});
  }, [api]);

  const handleToggle = async () => {
    const next = !yoloEnabled;
    try {
      await api.put('/settings/yolo', { enabled: next });
      setYoloEnabled(next);
    } catch (err) {
      console.error('Failed to toggle YOLO mode:', err);
    }
  };

  const handleSetMaxCronGen = async (val: number) => {
    const clamped = Math.max(0, Math.min(6, val));
    try {
      await api.put('/settings/max-cron-generation', { value: clamped });
      setMaxCronGen(clamped);
    } catch (err) {
      console.error('Failed to set max cron generation:', err);
    }
  };

  return (
    <>
      <SectionCard title="YOLO Mode">
        <div className="space-y-4">
          {!loading && (
            <Toggle enabled={yoloEnabled} onToggle={handleToggle} label="Auto-approve MCP permissions" />
          )}
          {yoloEnabled && (
            <div className="flex items-start gap-2 p-3 rounded-lg bg-amber-500/10 border border-amber-500/30">
              <AlertTriangle size={14} className="text-amber-400 mt-0.5 shrink-0" />
              <div className="space-y-1">
                <p className="text-[10px] font-bold text-amber-400 uppercase tracking-widest">Warning</p>
                <p className="text-[10px] text-content-tertiary">MCP server connection permissions are auto-approved without manual review. Tool execution still requires approval. SafetyGate and code validation remain active.</p>
              </div>
            </div>
          )}
          {!yoloEnabled && (
            <p className="text-[10px] text-content-tertiary">When enabled, MCP server connection permission requests are automatically approved. Tool execution approval is unaffected. SafetyGate post-validation remains active as a safety net.</p>
          )}
        </div>
      </SectionCard>

      <SectionCard title="CRON Recursion Limit">
        <div className="space-y-3">
          <p className="text-[10px] text-content-tertiary">
            Maximum generations a CRON job can recursively create child CRON jobs. 0 disables recursion entirely.
          </p>
          <div className="flex items-center gap-3">
            <input
              type="number"
              min={0}
              max={6}
              value={maxCronGen}
              onChange={e => handleSetMaxCronGen(Number(e.target.value))}
              className="w-16 bg-surface-secondary border border-edge rounded px-2 py-1 text-xs font-mono text-content-primary"
            />
            <span className="text-[10px] text-content-tertiary">(0-6, default: 2)</span>
          </div>
        </div>
      </SectionCard>
    </>
  );
}
