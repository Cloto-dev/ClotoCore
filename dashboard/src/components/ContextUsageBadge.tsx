import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../hooks/useApi';
import { compactCount } from '../lib/chatTime';

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
 * The context meter in the composer's row (docs/gui/samples/02-chat-conversation.html):
 * the most recent response's prompt_tokens against the provider's configured
 * context_length, as `18.9k / 128k`. Hidden entirely when nothing has been
 * recorded yet (avoids a flash of "0 / ?").
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

  // The colour shifts once the last response was within 20%/5% of the window.
  const tone = pct === null ? '' : pct >= 0.95 ? ' alert' : pct >= 0.8 ? ' warn' : '';

  const usedLabel = usage.is_estimate ? `~${compactCount(used)}` : compactCount(used);
  const maxLabel = max != null ? compactCount(max) : '?';

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
    <span
      role="status"
      className={`meter num${tone}`}
      title={tooltipParts}
      aria-label={t('agent.context_usage_aria', { used, max: maxLabel })}
    >
      {usedLabel} / {maxLabel}
    </span>
  );
}
