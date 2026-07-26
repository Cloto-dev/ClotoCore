import { ChevronDown, ChevronRight, Loader2, ShieldAlert } from 'lucide-react';
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
import { AlertCard } from '../ui/AlertCard';
import { ConfirmDialog } from '../ui/ConfirmDialog';
import { SecretInput } from '../ui/SecretInput';

/** Serde tag → the cumulative level the kernel accepts as `tier` (1..4). */
const TIER_LEVEL: Record<PurgeTierName, number> = {
  application: 1,
  user_data: 2,
  assets: 3,
  everything: 4,
};

const TIER_LEVELS = [1, 2, 3, 4] as const;

function Flag({ label, tone }: { label: string; tone: 'danger' | 'warn' }) {
  const classes =
    tone === 'danger'
      ? 'bg-red-500/10 border-red-500/30 text-red-400'
      : 'bg-amber-500/10 border-amber-500/30 text-amber-400';
  return <span className={`px-1.5 py-px rounded border text-[9px] font-mono ${classes}`}>{label}</span>;
}

function EntryRow({ entry }: { entry: PurgeEntry }) {
  const { t } = useTranslation('settings');

  const size =
    entry.size_bytes === undefined
      ? t('health.danger.size_unknown')
      : entry.size_truncated
        ? t('health.danger.size_lower_bound', { size: formatBytes(entry.size_bytes) })
        : formatBytes(entry.size_bytes);

  return (
    <div className="flex items-start gap-2 py-1.5 border-b border-edge last:border-b-0">
      <span className="w-12 shrink-0 pt-px text-[9px] font-mono uppercase tracking-wider text-content-tertiary">
        {entry.kind}
      </span>
      <div className="min-w-0 flex-1">
        <p className="text-[11px] font-mono text-content-primary break-all">{entry.path ?? entry.name ?? entry.id}</p>
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 mt-0.5">
          <span className="text-[9px] font-mono text-content-tertiary">
            {t('health.danger.tier_short', { level: TIER_LEVEL[entry.tier] })}
          </span>
          <span className="text-[9px] text-content-tertiary">{t(`health.danger.source_${entry.source}`)}</span>
          {entry.secret && <Flag tone="danger" label={t('health.danger.flag_secret')} />}
          {entry.covers_secret && <Flag tone="danger" label={t('health.danger.flag_covers_secret')} />}
          {entry.unreadable && <Flag tone="warn" label={t('health.danger.flag_unreadable')} />}
        </div>
      </div>
      <span className="shrink-0 pt-px text-[10px] font-mono text-content-tertiary">{size}</span>
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
    <div className="mt-6 pt-5 border-t border-edge">
      <div className="bg-glass backdrop-blur-sm border border-red-500/30 rounded-lg p-4">
        <div className="flex items-center gap-2 mb-2">
          <ShieldAlert size={14} className="text-red-400 shrink-0" />
          <h4 className="text-[11px] font-black uppercase tracking-[0.2em] text-red-400">{t('health.danger.title')}</h4>
        </div>

        {handoff ? (
          /* Post-handoff: the kernel exits on its own. Nothing to poll. */
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <Loader2 size={14} className="animate-spin text-red-400 shrink-0" />
              <span className="text-xs font-bold text-content-primary">{t('health.danger.running_title')}</span>
            </div>
            <p className="text-xs text-content-secondary">{t('health.danger.running_desc')}</p>
            <div>
              <span className="text-[9px] font-bold uppercase tracking-wider text-content-tertiary">
                {t('health.danger.report_path')}
              </span>
              <p className="text-[11px] font-mono text-content-primary break-all select-all">{handoff.report_path}</p>
            </div>
            <p className="text-[10px] font-mono text-content-tertiary leading-relaxed">
              {t('health.danger.running_resume_hint')}
            </p>
          </div>
        ) : (
          <>
            <p className="text-xs text-content-secondary mb-3">{t('health.danger.desc')}</p>

            {!open ? (
              <button
                type="button"
                onClick={handleOpen}
                className="px-4 py-2 text-xs font-bold uppercase tracking-widest rounded-lg bg-glass-subtle backdrop-blur-sm border border-edge hover:border-red-500 text-red-400 transition-colors"
              >
                {t('health.danger.review')}
              </button>
            ) : (
              <div className="space-y-4">
                {/* ── Gate 2: scope (cumulative tiers, default = narrowest) ── */}
                <div>
                  <p className="text-[9px] font-bold uppercase tracking-wider text-content-tertiary mb-1">
                    {t('health.danger.scope_title')}
                  </p>
                  <p className="text-[10px] text-content-tertiary mb-2">{t('health.danger.scope_hint')}</p>
                  <div className="space-y-1">
                    {TIER_LEVELS.map((level) => {
                      const included = level <= tier;
                      return (
                        <label
                          key={level}
                          className={`flex items-start gap-2 p-2 rounded border transition-colors ${
                            included ? 'border-red-500/30 bg-red-500/5' : 'border-edge'
                          } ${level === 1 || busy ? 'cursor-default' : 'cursor-pointer hover:border-red-500'}`}
                        >
                          <input
                            type="checkbox"
                            checked={included}
                            disabled={level === 1 || busy}
                            onChange={() => toggleTier(level)}
                            className="mt-0.5 accent-red-500"
                          />
                          <span className="min-w-0">
                            <span className="block text-[11px] font-bold text-content-primary">
                              {t(`health.danger.tier${level}`)}
                              {level === 1 && (
                                <span className="ml-1 font-normal text-content-tertiary">
                                  ({t('health.danger.tier1_always')})
                                </span>
                              )}
                            </span>
                            <span className="block text-[10px] text-content-tertiary">
                              {t(`health.danger.tier${level}_hint`)}
                            </span>
                          </span>
                        </label>
                      );
                    })}
                  </div>
                </div>

                {planAction.error && <AlertCard>{planAction.error}</AlertCard>}

                {/* ── Gate 1: the dry-run enumeration, rendered as real paths ── */}
                {planAction.isLoading && !plan ? (
                  <div className="flex items-center gap-2 py-6 justify-center text-content-tertiary">
                    <Loader2 size={14} className="animate-spin" />
                    <span className="text-xs">{t('health.danger.reviewing')}</span>
                  </div>
                ) : plan ? (
                  <div className={planAction.isLoading ? 'opacity-50' : undefined}>
                    <div className="flex items-center gap-2 mb-1">
                      <p className="text-[9px] font-bold uppercase tracking-wider text-content-tertiary">
                        {t('health.danger.plan_title')}
                      </p>
                      {planAction.isLoading && <Loader2 size={10} className="animate-spin text-content-tertiary" />}
                    </div>
                    <p className="text-[9px] font-mono text-content-tertiary">
                      {t('health.danger.plan_meta', {
                        planVersion: plan.plan.plan_version,
                        appVersion: plan.plan.app_version,
                        generatedAt: new Date(plan.plan.generated_at).toLocaleString(),
                      })}
                    </p>
                    <p className="text-[9px] font-mono text-content-tertiary break-all mb-2">
                      {t('health.danger.data_dir')}: {plan.plan.data_dir}
                    </p>

                    {/* Summary the kernel derived (totals skip size-less entries,
                        elevation is the executor's own rule). */}
                    <div className="flex flex-wrap items-center gap-2 mb-2">
                      <span className="text-[10px] font-mono text-content-secondary">
                        {t('health.danger.summary_items', { count: plan.summary.entries })} · {totalSize}
                      </span>
                      {plan.summary.contains_secret && <Flag tone="danger" label={t('health.danger.summary_secret')} />}
                      {plan.summary.needs_elevation && (
                        <Flag tone="warn" label={t('health.danger.summary_elevation')} />
                      )}
                    </div>
                    {plan.summary.total_truncated && (
                      <p className="text-[10px] text-amber-400 mb-2">{t('health.danger.summary_truncated')}</p>
                    )}

                    {plan.plan.entries.length === 0 ? (
                      <p className="text-[11px] text-amber-400 py-2">{t('health.danger.empty')}</p>
                    ) : (
                      <div className="max-h-64 overflow-y-auto pr-1">
                        {plan.plan.entries.map((entry) => (
                          <EntryRow key={`${entry.id}:${entry.path ?? entry.name ?? ''}`} entry={entry} />
                        ))}
                      </div>
                    )}

                    {/* Skipped candidates: "we looked and it was not there" is
                        part of the enumeration's trustworthiness (§7). */}
                    {plan.plan.skipped.length > 0 && (
                      <div className="mt-2">
                        <button
                          type="button"
                          onClick={() => setShowSkipped((v) => !v)}
                          className="flex items-center gap-1 text-[10px] text-content-tertiary hover:text-content-secondary transition-colors"
                        >
                          {showSkipped ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
                          {t('health.danger.skipped_show', { count: plan.plan.skipped.length })}
                        </button>
                        {showSkipped && (
                          <div className="mt-1 pl-3 border-l border-edge">
                            <p className="text-[9px] text-content-tertiary mb-1">{t('health.danger.skipped_hint')}</p>
                            {plan.plan.skipped.map((s) => (
                              <div
                                key={`${s.id}:${s.path ?? ''}`}
                                className="flex items-start gap-2 py-0.5 text-[10px] font-mono text-content-tertiary"
                              >
                                <span className="break-all flex-1">{s.path ?? s.id}</span>
                                <span className="shrink-0">{t(`health.danger.skip_${s.reason}`)}</span>
                              </div>
                            ))}
                          </div>
                        )}
                      </div>
                    )}

                    {/* Verbatim, every surface (§7 "Boundaries"). */}
                    {plan.plan.notes.length > 0 && (
                      <div className="mt-3">
                        <p className="text-[9px] font-bold uppercase tracking-wider text-content-tertiary mb-1">
                          {t('health.danger.notes_title')}
                        </p>
                        <ul className="list-disc pl-4 space-y-0.5">
                          {plan.plan.notes.map((note) => (
                            <li key={note} className="text-[10px] text-content-tertiary leading-relaxed">
                              {note}
                            </li>
                          ))}
                        </ul>
                      </div>
                    )}
                  </div>
                ) : null}

                {/* The enumeration on screen is for another scope: say so, and
                    keep gate 3 out of reach until a matching plan is read. */}
                {plan && !scopeMatchesPlan && !planAction.isLoading && (
                  <AlertCard variant="warning">{t('health.danger.scope_stale')}</AlertCard>
                )}

                {/* ── Gate 3: sudo mode ── */}
                {summary && summary.entries > 0 && scopeMatchesPlan && (
                  <div className="pt-3 border-t border-edge space-y-2">
                    <p className="text-[9px] font-bold uppercase tracking-wider text-content-tertiary">
                      {t('health.danger.sudo_title')}
                    </p>
                    <p className="text-[10px] text-content-tertiary leading-relaxed">{t('health.danger.sudo_desc')}</p>
                    <p className="text-[10px] text-content-tertiary">{t('health.danger.sudo_where')}</p>
                    <div className="flex gap-2">
                      <SecretInput
                        value={sudoKey}
                        onChange={(v) => {
                          setSudoKey(v);
                          execAction.clearError();
                        }}
                        placeholder={t('health.danger.sudo_placeholder')}
                        className="w-full bg-glass-strong backdrop-blur-sm border border-edge rounded-lg px-3 py-2 pr-8 text-xs font-mono text-content-primary placeholder:text-content-tertiary focus:outline-none focus:border-red-500 transition-colors"
                      />
                    </div>
                    {execAction.error && (
                      <AlertCard>
                        <span className="block">{execAction.error}</span>
                        <span className="block mt-1">{t('health.danger.error_ambiguous')}</span>
                      </AlertCard>
                    )}
                  </div>
                )}

                <div className="flex gap-2">
                  <button
                    type="button"
                    onClick={handleClose}
                    disabled={execAction.isLoading}
                    className="px-4 py-2 text-xs font-bold uppercase tracking-widest rounded-lg bg-glass-subtle backdrop-blur-sm border border-edge hover:border-brand text-content-secondary transition-colors disabled:opacity-40"
                  >
                    {t('health.danger.close')}
                  </button>
                  <button
                    type="button"
                    onClick={() => loadPlan(tier)}
                    disabled={busy}
                    className="px-4 py-2 text-xs font-bold uppercase tracking-widest rounded-lg bg-glass-subtle backdrop-blur-sm border border-edge hover:border-brand text-content-secondary transition-colors disabled:opacity-40"
                  >
                    {t('health.danger.refresh')}
                  </button>
                  <button
                    type="button"
                    onClick={() => setConfirming(true)}
                    disabled={!canExecute}
                    className="px-4 py-2 text-xs font-bold uppercase tracking-widest rounded-lg bg-red-500/10 border border-red-500/30 hover:border-red-500 text-red-400 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                  >
                    {execAction.isLoading ? (
                      <span className="flex items-center gap-2">
                        <Loader2 size={12} className="animate-spin" />
                        {t('health.danger.execute')}
                      </span>
                    ) : (
                      t('health.danger.execute')
                    )}
                  </button>
                </div>
              </div>
            )}
          </>
        )}
      </div>

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
