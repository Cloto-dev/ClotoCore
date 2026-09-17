import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { formatBytes } from '../../lib/format';
import type { HealthCheck, HealthReport, RepairReport } from '../../services/api';
import { SettingsGroup, SettingsRow } from './common';
import { DangerZone } from './DangerZone';

/** The check's state, as a dot before its words (the workshop's `.st`). */
function statusClass(status: string): string {
  switch (status) {
    case 'healthy':
      return 'st ok';
    case 'degraded':
      return 'st warn';
    case 'error':
      return 'st bad';
    default:
      return 'st';
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
    <>
      <SettingsGroup title={t('health.title')}>
        {report && (
          <p className="gdesc">
            {t('health.last_scan')}: {new Date(report.timestamp).toLocaleString()}
          </p>
        )}

        {error && <p className="says bad">{error}</p>}
        {repairResult && repairResult.total_fixed > 0 && (
          <p className="says ok">{t('health.repaired', { count: repairResult.total_fixed })}</p>
        )}

        {scanning && !report ? (
          <p className="says">{t('health.scanning')}</p>
        ) : report ? (
          <ul className="slist">
            {report.checks.map((check) => (
              <li key={check.name}>
                <span className="t">
                  <span className={statusClass(check.status)}>{check.message}</span>
                </span>
                {/* The action the check implies. Shown only where the kernel
                    actually observed the problem, so an installation with
                    nothing to update is never told to update anything. */}
                {staleConnectors(check).map((id) => (
                  <button
                    key={id}
                    type="button"
                    className="btn"
                    onClick={() => handleUpdateConnector(id)}
                    disabled={updating !== null}
                  >
                    {t('health.update_connector', { id })}
                  </button>
                ))}
              </li>
            ))}
          </ul>
        ) : null}

        {updated.length > 0 && <p className="says">{t('health.updated_connectors', { ids: updated.join(', ') })}</p>}

        {report && (
          <SettingsRow label={t('health.db_size')}>
            <span className="val num">{formatBytes(report.db_size_bytes)}</span>
          </SettingsRow>
        )}

        <div className="set-block">
          <button type="button" className="btn" onClick={() => loadReport(true)} disabled={scanning}>
            {t('health.scan')}
          </button>
          <button
            type="button"
            className="btn"
            onClick={handleRepair}
            disabled={repairing || repairableCount === 0}
            title={t('health.repair')}
          >
            {t('health.repair')}
          </button>
        </div>
      </SettingsGroup>

      {/* Complete uninstall, three gates (docs/DEFENDER_DESIGN.md §7). Appended
          at the bottom per the established Settings → Danger Zone grammar. */}
      <DangerZone />
    </>
  );
}
