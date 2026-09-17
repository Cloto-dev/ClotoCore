import { useTranslation } from 'react-i18next';
import { agentAccentTriplet } from '../lib/agentIdentity';
import type { ExternalAction } from '../types';
import { Ask } from './Ask';

interface ExternalActionCardProps {
  action: ExternalAction;
}

/**
 * Someone outside — a Discord member, a bridge — speaking to an agent, drawn
 * as a question from them: where it came from and who said it, their words,
 * and the agent's answer as the sentence below.
 */
export function ExternalActionCard({ action }: ExternalActionCardProps) {
  const { t } = useTranslation('agents');
  const isError = action.status === 'error';
  const isPending = action.status === 'pending';
  return (
    <div style={{ '--agent': agentAccentTriplet({ id: action.target_agent_id }) } as React.CSSProperties}>
      <Ask
        first
        who={t('ask.external_who', {
          source: action.source_label || action.source,
          from: action.sender_name,
          to: action.target_agent_name,
        })}
        question={action.prompt}
        why={
          isPending
            ? t('ask.awaiting', { name: action.target_agent_name })
            : isError
              ? t('ask.failed', { name: action.target_agent_name, error: action.response ?? '' })
              : (action.response ?? '')
        }
        whyTone={isError ? 'bad' : 'plain'}
      />
    </div>
  );
}
