import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../hooks/useApi';

type Usage = {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  context_length: number | null;
  provider_id: string;
  model_id: string;
  is_estimate: boolean;
  updated_at: string;
};

type Props = {
  agentId: string;
  /** Changing this value triggers a refetch — e.g. the message-count so the badge
   * refreshes after every new assistant turn. */
  refreshKey: number | string;
};

/**
 * Compact header chip showing the most recent response's prompt_tokens
 * vs the provider's configured context_length. Hidden entirely when nothing
 * has been recorded yet (avoids a flash of "0 / ?").
 */
export function ContextUsageBadge({ agentId, refreshKey }: Props) {
  const api = useApi();
  const { t } = useTranslation();
  const [usage, setUsage] = useState<Usage | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .getAgentLastUsage(agentId)
      .then((d) => {
        if (!cancelled) setUsage(d.usage);
      })
      .catch(() => {
        if (!cancelled) setUsage(null);
      });
    return () => {
      cancelled = true;
    };
  }, [api, agentId, refreshKey]);

  if (!usage) return null;

  const used = usage.prompt_tokens;
  const max = usage.context_length ?? null;
  const pct = max && max > 0 ? used / max : null;

  // Progressive severity — the whole badge shifts to warn/alert once the last
  // response was within 5%/20% of the configured context window.
  const tone =
    pct === null
      ? 'text-content-tertiary border-edge'
      : pct >= 0.95
        ? 'text-red-400 border-red-500/40'
        : pct >= 0.8
          ? 'text-amber-400 border-amber-500/40'
          : 'text-content-secondary border-edge';

  const usedLabel = usage.is_estimate ? `~${used.toLocaleString()}` : used.toLocaleString();
  const maxLabel = max != null ? max.toLocaleString() : '?';

  const tooltipParts = [
    `${usage.provider_id} · ${usage.model_id || '(no model)'}`,
    t('agent.context_usage_tooltip_prompt', { tokens: usage.prompt_tokens.toLocaleString() }),
    t('agent.context_usage_tooltip_completion', {
      tokens: usage.completion_tokens.toLocaleString(),
    }),
    usage.is_estimate ? t('agent.context_usage_tooltip_estimate') : null,
    new Date(usage.updated_at).toLocaleTimeString(),
  ]
    .filter(Boolean)
    .join('\n');

  return (
    <div
      className={`px-2.5 py-1 rounded-full border text-[10px] font-mono flex items-center gap-2 ${tone}`}
      title={tooltipParts}
      aria-label={t('agent.context_usage_aria', { used, max: maxLabel })}
    >
      <span>
        {usedLabel} / {maxLabel} tok
      </span>
      {pct !== null && (
        <span className="w-16 h-1 rounded-full bg-surface-secondary overflow-hidden">
          <span
            className="block h-full bg-current transition-all"
            style={{ width: `${Math.min(100, Math.round(pct * 100))}%` }}
          />
        </span>
      )}
    </div>
  );
}
