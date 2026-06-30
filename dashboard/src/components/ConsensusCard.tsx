import { CheckCircle2, ChevronDown, ChevronRight, Loader2, Sparkles, XCircle } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ConsensusRound, ConsensusStep } from '../types';

interface ConsensusCardProps {
  round: ConsensusRound;
}

function truncate(text: string, lines: number): { truncated: string; isTruncated: boolean } {
  const parts = text.split('\n');
  if (parts.length <= lines) return { truncated: text, isTruncated: false };
  return { truncated: parts.slice(0, lines).join('\n'), isTruncated: true };
}

function StatusIcon({ status }: { status: ConsensusStep['status'] }) {
  if (status === 'pending') return <Loader2 size={11} className="text-brand animate-spin shrink-0" />;
  if (status === 'error') return <XCircle size={11} className="text-red-400 shrink-0" />;
  return <CheckCircle2 size={11} className="text-emerald-400 shrink-0" />;
}

function StepRow({ step, defaultLines }: { step: ConsensusStep; defaultLines: number }) {
  const [expanded, setExpanded] = useState(false);
  const { t } = useTranslation('actions');
  const isError = step.status === 'error';
  const isPending = step.status === 'pending';
  const body = step.response ? truncate(step.response, expanded ? 999 : defaultLines) : null;

  return (
    <div
      className={`rounded-md border p-2 ${isError ? 'border-red-500/40 bg-red-500/5' : 'border-edge bg-glass-strong'}`}
    >
      <div className="flex items-center justify-between gap-2 mb-1">
        <div className="flex items-center gap-1.5 min-w-0">
          <StatusIcon status={step.status} />
          <span className="text-[10px] font-mono font-bold uppercase tracking-wider text-content-secondary truncate">
            {step.engine_id.replace('mind.', '')}
          </span>
        </div>
        {isError && step.mgp_error_code != null && (
          <span className="text-[9px] font-mono text-red-400 bg-red-500/10 px-1.5 py-0.5 rounded shrink-0">
            MGP-{step.mgp_error_code}
            {step.retryable ? ` · ${t('consensusCard.retryable')}` : ''}
          </span>
        )}
      </div>

      {isPending ? (
        <span className="text-[10px] text-content-tertiary animate-pulse">{t('consensusCard.thinking')}</span>
      ) : (
        body && (
          <div
            className={`text-[11px] whitespace-pre-wrap break-words leading-relaxed ${
              isError ? 'text-red-400' : 'text-content-primary'
            }`}
          >
            {body.truncated}
            {!expanded && body.isTruncated && <span className="text-content-tertiary">...</span>}
            {body.isTruncated && (
              <button
                type="button"
                onClick={() => setExpanded(!expanded)}
                className="ml-1 inline-flex items-center gap-0.5 text-[9px] font-bold uppercase tracking-wider text-content-tertiary hover:text-content-secondary transition-colors align-middle"
              >
                {expanded ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
                {expanded ? t('consensusCard.collapse') : t('consensusCard.expand')}
              </button>
            )}
          </div>
        )
      )}
    </div>
  );
}

export function ConsensusCard({ round }: ConsensusCardProps) {
  const { t } = useTranslation('actions');
  const proposals = round.steps.filter((s) => s.phase === 'proposal');
  const synthesis = round.steps.find((s) => s.phase === 'synthesis');
  const prompt = truncate(round.prompt, 2);

  return (
    <div className="rounded-lg border border-edge bg-glass-subtle p-3">
      {/* Header: consensus badge → agent */}
      <div className="flex items-center justify-between gap-2 mb-2">
        <div className="flex items-center gap-1.5 min-w-0">
          <span className="inline-flex items-center gap-1 text-[9px] font-bold uppercase tracking-wider px-1.5 py-0.5 rounded shrink-0 bg-brand/15 text-brand">
            <Sparkles size={10} />
            {t('tabs.consensus')}
          </span>
          <span className="text-[10px] font-mono font-bold uppercase tracking-wider text-content-secondary truncate">
            {round.agent_name}
          </span>
        </div>
        <span className="text-[9px] font-mono text-content-tertiary bg-surface-secondary px-1.5 py-0.5 rounded shrink-0">
          {proposals.length} {t('consensusCard.engines')}
        </span>
      </div>

      {/* Prompt */}
      <div className="text-[11px] text-content-primary whitespace-pre-wrap break-words leading-relaxed mb-2">
        {prompt.truncated}
        {prompt.isTruncated && <span className="text-content-tertiary">...</span>}
      </div>

      {/* Proposals */}
      {proposals.length > 0 && (
        <>
          <div className="text-[9px] font-bold uppercase tracking-wider text-content-tertiary mb-1">
            {t('consensusCard.proposals')}
          </div>
          <div className="space-y-1.5 mb-2">
            {proposals.map((s) => (
              <StepRow key={`proposal-${s.engine_id}`} step={s} defaultLines={3} />
            ))}
          </div>
        </>
      )}

      {/* Synthesis */}
      {synthesis && (
        <>
          <div className="text-[9px] font-bold uppercase tracking-wider text-brand/80 mb-1">
            {t('consensusCard.synthesis')}
          </div>
          <StepRow key="synthesis" step={synthesis} defaultLines={8} />
        </>
      )}

      <div className="mt-2 flex justify-end">
        <span className="text-[10px] font-mono text-content-tertiary">
          {new Date(round.timestamp).toLocaleTimeString()}
        </span>
      </div>
    </div>
  );
}
