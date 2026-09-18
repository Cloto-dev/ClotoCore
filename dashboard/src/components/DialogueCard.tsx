import { useTranslation } from 'react-i18next';
import { agentAccentTriplet } from '../lib/agentIdentity';
import type { AgentDialogue } from '../types';
import { Ask } from './Ask';

interface DialogueCardProps {
  dialogue: AgentDialogue;
}

/**
 * One agent asking another, drawn as a question from the caller: the caller's
 * colour on the edge, their words, and the target's answer as the sentence
 * below — or that it is still being waited for.
 */
export function DialogueCard({ dialogue }: DialogueCardProps) {
  const { t } = useTranslation('agents');
  const isError = dialogue.status === 'error';
  const isPending = dialogue.status === 'pending';
  return (
    <div
      style={{ '--agent': agentAccentTriplet({ id: dialogue.caller_agent_id }) } as React.CSSProperties}
      className={dialogue.chain_depth > 1 ? 'ml-4' : ''}
    >
      <Ask
        first
        who={t('ask.dialogue_who', { from: dialogue.caller_agent_name, to: dialogue.target_agent_name })}
        question={dialogue.prompt}
        why={
          isPending
            ? t('ask.awaiting', { name: dialogue.target_agent_name })
            : isError
              ? t('ask.failed', { name: dialogue.target_agent_name, error: dialogue.response ?? '' })
              : (dialogue.response ?? '')
        }
        whyTone={isError ? 'bad' : 'plain'}
      />
    </div>
  );
}
