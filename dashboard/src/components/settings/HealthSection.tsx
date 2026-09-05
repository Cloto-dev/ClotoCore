import { AlertTriangle, CheckCircle, Download, Loader2, XCircle } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { formatBytes } from '../../lib/format';
import type { HealthCheck, HealthReport, RepairReport } from '../../services/api';
import { SectionCard } from './common';
import { DangerZone } from './DangerZone';

function StatusIcon({ status }: { status: string }) {
  switch (status) {
    case 'healthy':
      return <CheckCircle size={14} className="text-green-500 shrink-0" />;
    case 'degraded':
      return <AlertTriangle size={14} className="text-amber-400 shrink-0" />;
    case 'error':
      return <XCircle size={14} className="text-red-500 shrink-0" />;
    default:
      return null;
  }
}

export function HealthSection() {
  const { t } = useTranslation('settings');
  const api = useApi();
  const [report, setReport] = useState<HealthReport | null>(null);
  const [repairResult, setRepairResult] = useState<RepairReport | null>(null);
  const [scanning, setScanning] = useState(false);
  const [repairing, setRepairing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [updating, setUpdating] = useState<string | null>(null);
  const [updated, setUpdated] = useState<string[]>([]);

  const loadReport = useCallback(
    async (fresh?: boolean) => {
      try {
        setScanning(true);
        setError(null);
        setRepairResult(null);
        const data = await api.scanHealth(fresh);
        setReport(data);
      } catch (e) {
        setError(e instanceof Error ? e.message : 'Scan failed');
      } finally {
        setScanning(false);
      }
    },
    [api],
  );

  useEffect(() => {
    loadReport();
  }, [loadReport]);

  const handleRepair = async () => {
    try {
      setRepairing(true);
      setError(null);
      const result = await api.repairHealth();
      setRepairResult(result);
      // Re-scan to reflect repairs
      await loadReport(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Repair failed');
    } finally {
      setRepairing(false);
    }
  };

  const repairableCount = report?.checks.filter((c) => c.repairable).length ?? 0;

  // Connectors the LLM proxy saw calling without this kernel's token. They are
  // named by the check itself, from provider rows it resolved — the marketplace
  // cannot find them on its own, because it compares version strings and these
  // connectors changed content under an unchanged version.
  const staleConnectors = (check: HealthCheck): string[] => {
    if (check.name !== 'llm_proxy_untrusted_callers') return [];
    const ids = check.detail?.stale_connectors;
    return Array.isArray(ids) ? ids.filter((id): id is string => typeof id === 'string') : [];
  };

  // Updating is the same operation the marketplace's Update button performs:
  // stop the child, re-vendor from the catalog, restart it. The kernel drops
  // the connector from the check's list once that succeeds, so the re-scan
  // below is what makes the row shrink.
  const handleUpdateConnector = async (serverId: string) => {
    try {
      setUpdating(serverId);
      setError(null);
      await api.installMarketplaceServer({ server_id: serverId, update: true });
      setUpdated((done) => (done.includes(serverId) ? done : [...done, serverId]));
      await loadReport(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : `Update of ${serverId} failed`);
    } finally {
      setUpdating(null);
    }
  };

  return (
    <SectionCard title={t('health.title')}>
      {/* Last scan timestamp */}
      {report && (
        <p className="text-xs text-content-tertiary mb-4">
          {t('health.last_scan')}: {new Date(report.timestamp).toLocaleString()}
        </p>
      )}

      {/* Error banner */}
      {error && (
        <div className="flex items-center gap-2 p-3 rounded-lg bg-red-500/10 border border-red-500/30 mb-4">
          <XCircle size={14} className="text-red-400 shrink-0" />
          <p className="text-xs text-red-400">{error}</p>
        </div>
      )}

      {/* Repair result banner */}
      {repairResult && repairResult.total_fixed > 0 && (
        <div className="flex items-center gap-2 p-3 rounded-lg bg-green-500/10 border border-green-500/30 mb-4">
          <CheckCircle size={14} className="text-green-400 shrink-0" />
          <p className="text-xs text-green-400">{t('health.repaired', { count: repairResult.total_fixed })}</p>
        </div>
      )}

      {/* Check results */}
      {scanning && !report ? (
        <div className="flex items-center gap-2 py-8 justify-center text-content-tertiary">
          <Loader2 size={16} className="animate-spin" />
          <span className="text-sm">Scanning...</span>
        </div>
      ) : report ? (
        <div className="space-y-2 mb-4">
          {report.checks.map((check) => (
            <div key={check.name} className="py-1.5">
              <div className="flex items-center gap-3">
                <StatusIcon status={check.status} />
                <span className="text-sm text-content-secondary flex-1">{check.message}</span>
              </div>
              {/* The action the check implies. Shown only where the kernel
                  actually observed the problem, so an installation with nothing
                  to update is never told to update anything. */}
              {staleConnectors(check).length > 0 && (
                <div className="flex flex-wrap items-center gap-2 mt-2 ml-[26px]">
                  {staleConnectors(check).map((id) => (
                    <button
                      key={id}
                      type="button"
                      onClick={() => handleUpdateConnector(id)}
                      disabled={updating !== null}
                      className="flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs border border-edge text-content-secondary hover:text-content-primary disabled:opacity-50"
                    >
                      {updating === id ? <Loader2 size={12} className="animate-spin" /> : <Download size={12} />}
                      {t('health.update_connector', { id })}
                    </button>
                  ))}
                </div>
              )}
            </div>
          ))}
          {updated.length > 0 && (
            <p className="text-xs text-content-tertiary ml-[26px]">
              {t('health.updated_connectors', { ids: updated.join(', ') })}
            </p>
          )}

          {/* DB size */}
          <div className="flex items-center gap-3 py-1.5 border-t border-edge mt-2 pt-3">
            <span className="text-xs text-content-tertiary">
              {t('health.db_size')}: {formatBytes(report.db_size_bytes)}
            </span>
          </div>
        </div>
      ) : null}

      {/* Action buttons */}
      <div className="flex gap-3 mt-2">
        <button
          type="button"
          onClick={() => loadReport(true)}
          disabled={scanning}
          className="px-4 py-2 text-xs font-bold uppercase tracking-widest rounded-lg bg-surface-secondary border border-edge hover:bg-surface-secondary/80 text-content-secondary transition-colors disabled:opacity-50"
        >
          {scanning ? (
            <span className="flex items-center gap-2">
              <Loader2 size={12} className="animate-spin" />
              {t('health.scan')}
            </span>
          ) : (
            t('health.scan')
          )}
        </button>
        <button
          type="button"
          onClick={handleRepair}
          disabled={repairing || repairableCount === 0}
          className="px-4 py-2 text-xs font-bold uppercase tracking-widest rounded-lg bg-brand/10 border border-brand/30 hover:bg-brand/20 text-brand transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
        >
          {repairing ? (
            <span className="flex items-center gap-2">
              <Loader2 size={12} className="animate-spin" />
              {t('health.repair')}
            </span>
          ) : (
            t('health.repair')
          )}
        </button>
      </div>

      {/* Complete uninstall, three gates (docs/DEFENDER_DESIGN.md §7). Appended
          at the bottom per the established Settings → Danger Zone grammar. */}
      <DangerZone />
    </SectionCard>
  );
}
