import { useTranslation } from 'react-i18next';
import type { PendingRejection } from '../types';
import { Ask } from './Ask';

interface Props {
  rejection: PendingRejection;
  onDismiss: (localId: string) => void;
  first?: boolean;
}

/**
 * The kernel refused one of the agent's tool calls. Drawn as a word from the
 * agent — what was refused, and what can be done next — with nothing to
 * answer: a rejection carries no "enable it" shortcut by design (the operator
 * changes settings under Settings → Security). "Close" is not an answer; it
 * only takes the line off the screen, and appears on hover like the message
 * acts do.
 */
export function ToolRejectionCard({ rejection, onDismiss, first = false }: Props) {
  const { t } = useTranslation('agents');
  const next = [rejection.remediation_hint, rejection.retryable ? null : t('ask.rejected_hard')]
    .filter(Boolean)
    .join(' ');
  return (
    <Ask
      first={first}
      question={t('ask.rejected', { tool: rejection.tool_name })}
      why={
        <>
          {rejection.reason}
          {next && (
            <>
              {'\n'}
              {next}
            </>
          )}
          {'\n'}
          <span className="num">{rejection.code}</span>
        </>
      }
      dismiss={{ label: t('ask.close'), onClick: () => onDismiss(rejection.local_id) }}
    />
  );
}
