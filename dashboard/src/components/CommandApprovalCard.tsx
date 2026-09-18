import type { ReactNode } from 'react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../hooks/useApi';
import { displayLevel } from '../lib/notificationSeverity';
import type { NotificationSeverity } from '../services/api';
import { Ask } from './Ask';

interface CommandEntry {
  command: string;
  command_name: string;
}

interface Props {
  approvalId: string;
  /** Who is asking. It used to be implied by the conversation the card sat in. */
  agentId?: string;
  /** The asking agent's name, when the surroundings do not already say it. */
  agentName?: string;
  commands: CommandEntry[];
  /** The kernel's derivation of what the commands could do. */
  severity?: NotificationSeverity;
  onResolved: (approvalId: string) => void;
  /** The deck's position indicator, shown where the countdown used to be. */
  pager?: ReactNode;
  /**
   * Hold the buttons briefly. The deck sets this after an answer, because the
   * next card's buttons land on the coordinates the last click was aimed at.
   */
  actionsDisabled?: boolean;
  /** Drawn as the first thing in its block: no top margin. */
  first?: boolean;
}

// The card used to count down from 60 and take itself off the screen at zero,
// mirroring a deadline the kernel no longer has. Leaving the countdown in would
// have been worse than a stale number: the timer called `onResolved`, so the one
// place to answer disappeared after a minute while the agent went on waiting.
// The card now stays until somebody answers it. The deck around it may stop
// *showing* it after a while — that is a different thing, and it does not
// resolve anything.
//
// The three answers, in the mock's words: "go ahead" trusts the command name
// from now on, "just this once" approves this request, "not now" denies it.
export function CommandApprovalCard({
  approvalId,
  agentId,
  agentName,
  commands,
  severity,
  onResolved,
  pager,
  actionsDisabled = false,
  first = false,
}: Props) {
  const api = useApi();
  const { t } = useTranslation('agents');
  const [status, setStatus] = useState<'pending' | 'acting' | 'resolved'>('pending');
  const busy = status === 'acting' || actionsDisabled;

  const handle = async (action: 'approve' | 'trust' | 'deny') => {
    setStatus('acting');
    try {
      if (action === 'approve') await api.approveCommand(approvalId);
      else if (action === 'trust') await api.trustCommand(approvalId);
      else await api.denyCommand(approvalId);
      setStatus('resolved');
      onResolved(approvalId);
    } catch (e) {
      if (import.meta.env.DEV) console.error('Command approval action failed:', e);
      setStatus('pending');
    }
  };

  if (status === 'resolved') return null;

  const uniqueNames = [...new Set(commands.map((c) => c.command_name))];
  const level = severity ? displayLevel(severity) : null;

  return (
    <Ask
      first={first}
      who={agentName || agentId ? <span data-testid="approval-agent">{agentName ?? agentId}</span> : undefined}
      question={commands.length > 1 ? t('ask.run_these', { count: commands.length }) : t('ask.run_this')}
      command={commands.map((c) => c.command)}
      why={
        uniqueNames.length === 1
          ? t('ask.run_why', { name: uniqueNames[0] })
          : t('ask.run_why_several', { count: uniqueNames.length })
      }
      ok={{ label: t('ask.go_ahead'), onClick: () => handle('trust') }}
      alts={[
        { label: t('ask.just_once'), onClick: () => handle('approve') },
        { label: t('ask.not_now'), onClick: () => handle('deny') },
      ]}
      actionsDisabled={busy}
      risk={level ? t('ask.risk', { level: t(`ask.risk_${level}`) }) : undefined}
      pager={pager}
    />
  );
}
