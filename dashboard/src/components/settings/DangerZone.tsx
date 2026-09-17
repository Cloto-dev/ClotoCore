import { useCallback, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { useAsyncAction } from '../../hooks/useAsyncAction';
import { formatBytes } from '../../lib/format';
import {
  api,
  type PurgeEntry,
  type PurgeTierName,
  type UninstallPlanResponse,
  type UninstallResponse,
} from '../../services/api';
import { ConfirmDialog } from '../ui/ConfirmDialog';
import { SecretInput } from '../ui/SecretInput';
import { SettingsGroup } from './common';

/** Serde tag → the cumulative level the kernel accepts as `tier` (1..4). */
const TIER_LEVEL: Record<PurgeTierName, number> = {
  application: 1,
  user_data: 2,
  assets: 3,
  everything: 4,
};

const TIER_LEVELS = [1, 2, 3, 4] as const;

function EntryRow({ entry }: { entry: PurgeEntry }) {
  const { t } = useTranslation('settings');

  const size =
    entry.size_bytes === undefined
      ? t('health.danger.size_unknown')
      : entry.size_truncated
        ? t('health.danger.size_lower_bound', { size: formatBytes(entry.size_bytes) })
        : formatBytes(entry.size_bytes);

  return (
    <div className="e">
      <span className="k">{entry.kind}</span>
      <span className="w">
        <span className="p">{entry.path ?? entry.name ?? entry.id}</span>
        <span className="m">
          <span>{t('health.danger.tier_short', { level: TIER_LEVEL[entry.tier] })}</span>
          <span>{t(`health.danger.source_${entry.source}`)}</span>
          {entry.secret && <span className="flag">{t('health.danger.flag_secret')}</span>}
          {entry.covers_secret && <span className="flag">{t('health.danger.flag_covers_secret')}</span>}
          {entry.unreadable && <span className="flag warn">{t('health.danger.flag_unreadable')}</span>}
        </span>
      </span>
      <span className="z">{size}</span>
    </div>
  );
}

/**
 * Settings → Health → Danger Zone (`docs/DEFENDER_DESIGN.md` §7).
 *
 * Three gates, in order: the dry-run enumeration (`GET
 * /api/system/uninstall/plan`, re-read whenever the scope widens), the scope
 * checkboxes (cumulative tiers, default = the narrowest), and sudo mode (the
 * admin key typed by hand — a deliberateness gate, not a security boundary;
 * the boundary is the kernel's own `X-API-Key` check).
 *
 * `POST /api/system/uninstall` is terminal: the kernel exits about a second
 * after a 200, so the success path renders the handoff (report path + "close
 * this window") and never polls or re-scans anything.
 */
export function DangerZone() {
  const { t } = useTranslation('settings');
  const { t: tc } = useTranslation();
  const authApi = useApi();

  const [open, setOpen] = useState(false);
  const [tier, setTier] = useState(1);
  const [plan, setPlan] = useState<UninstallPlanResponse | null>(null);
  const [showSkipped, setShowSkipped] = useState(false);
  const [sudoKey, setSudoKey] = useState('');
  const [confirming, setConfirming] = useState(false);
  const [handoff, setHandoff] = useState<UninstallResponse | null>(null);

  const planAction = useAsyncAction(t('health.danger.error_plan'));
  const execAction = useAsyncAction(t('health.danger.error_execute'));

  // Last-write-wins guard. The checkboxes are disabled while a plan is in
  // flight, so this is a second line of defence: a stale reply must never
  // repaint the list for a scope the user has already left.
  const requestSeq = useRef(0);

  const loadPlan = useCallback(
    (level: number) => {
      const seq = ++requestSeq.current;
      return planAction.run(async () => {
        const data = await authApi.getUninstallPlan(level);
        if (seq === requestSeq.current) setPlan(data);
      });
    },
    [authApi, planAction],
  );

  const handleOpen = () => {
    setOpen(true);
    loadPlan(tier);
  };

  const handleClose = () => {
    requestSeq.current += 1; // ignore whatever is still in flight
    setOpen(false);
    setPlan(null);
    setShowSkipped(false);
    setSudoKey('');
    setTier(1);
    planAction.clearError();
    execAction.clearError();
  };

  const selectTier = (level: number) => {
    if (planAction.isLoading || execAction.isLoading || level === tier) return;
    setTier(level);
    // The key was typed for the previous scope; widening it is a new decision.
    setSudoKey('');
    execAction.clearError();
    loadPlan(level);
  };

  /** Cumulative checkbox semantics: unchecking tier N lands on tier N-1. */
  const toggleTier = (level: number) => {
    if (level === 1) return; // the floor is always included
    selectTier(level <= tier ? level - 1 : level);
  };

  const handleExecute = () => {
    setConfirming(false);
    execAction.run(async () => {
      const result = await api.executeUninstall(sudoKey.trim(), { tier });
      // Terminal state: the kernel is on its way out. Do not re-scan.
      setSudoKey('');
      setHandoff(result);
    });
  };

  const summary = plan?.summary;
  const totalSize = summary
    ? summary.total_truncated
      ? t('health.danger.size_lower_bound', { size: formatBytes(summary.total_bytes) })
      : formatBytes(summary.total_bytes)
    : '';
  const busy = planAction.isLoading || execAction.isLoading;
  // Gate 1 only holds if the list on screen is the list for the scope that
  // would be executed. Widening the scope and having the re-read fail leaves
  // the narrower plan rendered — executing then would remove things the user
  // was never shown. The tier is read back from the plan the kernel returned,
  // not from a local mirror of what we asked for.
  const scopeMatchesPlan = !!plan && TIER_LEVEL[plan.plan.tier] === tier;
  const canExecute = !!summary && summary.entries > 0 && scopeMatchesPlan && !!sudoKey.trim() && !busy;

  return (
    <div className="set-danger">
      <SettingsGroup title={t('health.danger.title')}>
        {handoff ? (
          /* Post-handoff: the kernel exits on its own. Nothing to poll. */
          <div className="set-block">
            <p className="says bad">{t('health.danger.running_title')}</p>
            <p className="says">{t('health.danger.running_desc')}</p>
            <p className="says">
              {t('health.danger.report_path')}: <span className="select-all">{handoff.report_path}</span>
            </p>
            <p className="quote">{t('health.danger.running_resume_hint')}</p>
          </div>
        ) : (
          <>
            <p className="gdesc">{t('health.danger.desc')}</p>

            {!open ? (
              <div className="set-block">
                <button type="button" className="btn danger" onClick={handleOpen}>
                  {t('health.danger.review')}
                </button>
              </div>
            ) : (
              <div className="set-block">
                {/* ── Gate 2: scope (cumulative tiers, default = narrowest) ── */}
                <h2>{t('health.danger.scope_title')}</h2>
                <p className="gdesc">{t('health.danger.scope_hint')}</p>
                <div className="tiers">
                  {TIER_LEVELS.map((level) => {
                    const included = level <= tier;
                    return (
                      <label key={level} className={level === 1 || busy ? 'held' : undefined}>
                        <input
                          type="checkbox"
                          checked={included}
                          disabled={level === 1 || busy}
                          onChange={() => toggleTier(level)}
                        />
                        <span>
                          <span className="n">
                            {t(`health.danger.tier${level}`)}
                            {level === 1 && <span className="always">({t('health.danger.tier1_always')})</span>}
                          </span>
                          <span className="h">{t(`health.danger.tier${level}_hint`)}</span>
                        </span>
                      </label>
                    );
                  })}
                </div>

                {planAction.error && <p className="says bad">{planAction.error}</p>}

                {/* ── Gate 1: the dry-run enumeration, rendered as real paths ── */}
                {planAction.isLoading && !plan ? (
                  <p className="says">{t('health.danger.reviewing')}</p>
                ) : plan ? (
                  <div className={planAction.isLoading ? 'opacity-50' : undefined}>
                    <h2>{t('health.danger.plan_title')}</h2>
                    <p className="gdesc">
                      {t('health.danger.plan_meta', {
                        planVersion: plan.plan.plan_version,
                        appVersion: plan.plan.app_version,
                        generatedAt: new Date(plan.plan.generated_at).toLocaleString(),
                      })}
                    </p>
                    <p className="gdesc">
                      {t('health.danger.data_dir')}: {plan.plan.data_dir}
                    </p>
                    <p className="gdesc facts">
                      <span>{t('health.danger.summary_items', { count: plan.summary.entries })}</span>
                      <span>{totalSize}</span>
                      {plan.summary.contains_secret && (
                        <span className="flag">{t('health.danger.summary_secret')}</span>
                      )}
                      {plan.summary.needs_elevation && (
                        <span className="flag warn">{t('health.danger.summary_elevation')}</span>
                      )}
                    </p>
                    {plan.summary.total_truncated && (
                      <p className="says warn">{t('health.danger.summary_truncated')}</p>
                    )}

                    {plan.plan.entries.length === 0 ? (
                      <p className="says warn">{t('health.danger.empty')}</p>
                    ) : (
                      <div className="purge">
                        {plan.plan.entries.map((entry) => (
                          <EntryRow key={`${entry.id}:${entry.path ?? entry.name ?? ''}`} entry={entry} />
                        ))}
                      </div>
                    )}

                    {/* Skipped candidates: "we looked and it was not there" is
                        part of the enumeration's trustworthiness (§7). */}
                    {plan.plan.skipped.length > 0 && (
                      <div className="set-block">
                        <button type="button" className="btn" onClick={() => setShowSkipped((v) => !v)}>
                          {t('health.danger.skipped_show', { count: plan.plan.skipped.length })}
                        </button>
                        {showSkipped && (
                          <>
                            <p className="gdesc">{t('health.danger.skipped_hint')}</p>
                            <div className="purge">
                              {plan.plan.skipped.map((s) => (
                                <div className="e" key={`${s.id}:${s.path ?? ''}`}>
                                  <span className="p">{s.path ?? s.id}</span>
                                  <span className="z">{t(`health.danger.skip_${s.reason}`)}</span>
                                </div>
                              ))}
                            </div>
                          </>
                        )}
                      </div>
                    )}

                    {/* Verbatim, every surface (§7 "Boundaries"). */}
                    {plan.plan.notes.length > 0 && (
                      <>
                        <h2>{t('health.danger.notes_title')}</h2>
                        {plan.plan.notes.map((note) => (
                          <p className="gdesc" key={note}>
                            {note}
                          </p>
                        ))}
                      </>
                    )}
                  </div>
                ) : null}

                {/* The enumeration on screen is for another scope: say so, and
                    keep gate 3 out of reach until a matching plan is read. */}
                {plan && !scopeMatchesPlan && !planAction.isLoading && (
                  <p className="says warn">{t('health.danger.scope_stale')}</p>
                )}

                {/* ── Gate 3: sudo mode ── */}
                {summary && summary.entries > 0 && scopeMatchesPlan && (
                  <>
                    <h2>{t('health.danger.sudo_title')}</h2>
                    <p className="gdesc">{t('health.danger.sudo_desc')}</p>
                    <p className="gdesc">{t('health.danger.sudo_where')}</p>
                    <div className="set-block">
                      <SecretInput
                        value={sudoKey}
                        onChange={(v) => {
                          setSudoKey(v);
                          execAction.clearError();
                        }}
                        placeholder={t('health.danger.sudo_placeholder')}
                        className="in wide mono"
                      />
                    </div>
                    {execAction.error && (
                      <p className="says bad">
                        {execAction.error} {t('health.danger.error_ambiguous')}
                      </p>
                    )}
                  </>
                )}

                <div className="set-block">
                  <button type="button" className="btn" onClick={handleClose} disabled={execAction.isLoading}>
                    {t('health.danger.close')}
                  </button>
                  <button type="button" className="btn" onClick={() => loadPlan(tier)} disabled={busy}>
                    {t('health.danger.refresh')}
                  </button>
                  <button
                    type="button"
                    className="btn danger"
                    onClick={() => setConfirming(true)}
                    disabled={!canExecute}
                  >
                    {t('health.danger.execute')}
                  </button>
                </div>
              </div>
            )}
          </>
        )}
      </SettingsGroup>

      <ConfirmDialog
        open={confirming}
        title={t('health.danger.confirm_title')}
        message={t('health.danger.confirm_message', {
          tier,
          items: t('health.danger.summary_items', { count: summary?.entries ?? 0 }),
          size: totalSize,
        })}
        confirmLabel={t('health.danger.confirm_label')}
        cancelLabel={tc('cancel')}
        variant="danger"
        onConfirm={handleExecute}
        onCancel={() => setConfirming(false)}
      />
    </div>
  );
}
