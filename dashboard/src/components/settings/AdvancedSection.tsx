import { useState, useEffect } from 'react';
import { AlertTriangle } from 'lucide-react';
import { SectionCard, Toggle } from './common';
import { useApiKey } from '../../contexts/ApiKeyContext';
import { api } from '../../services/api';

export function AdvancedSection() {
  const { apiKey } = useApiKey();
  const [yoloEnabled, setYoloEnabled] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    api.fetchJson<{ enabled: boolean }>('/settings/yolo', apiKey)
      .then(data => setYoloEnabled(data.enabled))
      .catch(() => {})
      .finally(() => setLoading(false));
  }, [apiKey]);

  const handleToggle = async () => {
    const next = !yoloEnabled;
    try {
      await api.put('/settings/yolo', { enabled: next }, apiKey);
      setYoloEnabled(next);
    } catch (err) {
      console.error('Failed to toggle YOLO mode:', err);
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
                <p className="text-[10px] text-content-muted">MCP server permissions are auto-approved without manual review. SafetyGate and code validation remain active.</p>
              </div>
            </div>
          )}
          {!yoloEnabled && (
            <p className="text-[10px] text-content-muted">When enabled, MCP server permission requests are automatically approved. SafetyGate post-validation remains active as a safety net.</p>
          )}
        </div>
      </SectionCard>
    </>
  );
}
