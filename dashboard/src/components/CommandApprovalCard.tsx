import { Check, Shield, Terminal, X } from 'lucide-react';
import { useState } from 'react';
import { useApi } from '../hooks/useApi';
import { SystemAlertCard } from './SystemAlertCard';

interface CommandEntry {
  command: string;
  command_name: string;
}

interface Props {
  approvalId: string;
  commands: CommandEntry[];
  onResolved: (approvalId: string) => void;
}

// The card used to count down from 60 and take itself off the screen at zero,
// mirroring a deadline the kernel no longer has. Leaving the countdown in would
// have been worse than a stale number: the timer called `onResolved`, so the one
// place to answer disappeared after a minute while the agent went on waiting.
// The card now stays until somebody answers it.
export function CommandApprovalCard({ approvalId, commands, onResolved }: Props) {
  const api = useApi();
  const [status, setStatus] = useState<'pending' | 'acting' | 'resolved'>('pending');

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
  const trustLabel = uniqueNames.length === 1 ? `Trust '${uniqueNames[0]}'` : `Trust ${uniqueNames.length} commands`;

  return (
    <SystemAlertCard
      icon={<Terminal size={14} />}
      title={`Command Approval${commands.length > 1 ? ` (${commands.length})` : ''}`}
      footer={
        <div className="flex gap-2">
          <button
            onClick={() => handle('approve')}
            disabled={status === 'acting'}
            aria-label="Approve command"
            className="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[10px] font-bold uppercase tracking-wider text-emerald-400 hover:bg-emerald-500/10 disabled:opacity-50 transition-colors"
          >
            <Check size={12} /> Yes
          </button>
          <button
            onClick={() => handle('trust')}
            disabled={status === 'acting'}
            aria-label={trustLabel}
            className="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[10px] font-bold uppercase tracking-wider text-amber-500 hover:bg-amber-500/10 disabled:opacity-50 transition-colors"
          >
            <Shield size={12} /> {trustLabel}
          </button>
          <button
            onClick={() => handle('deny')}
            disabled={status === 'acting'}
            aria-label="Deny command"
            className="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[10px] font-bold uppercase tracking-wider text-content-secondary hover:text-red-400 hover:bg-red-500/10 disabled:opacity-50 transition-colors"
          >
            <X size={12} /> No
          </button>
        </div>
      }
    >
      {/* Command display */}
      <div className="font-mono text-xs text-content-secondary space-y-1 whitespace-pre-line">
        {commands.map((cmd, i) => (
          <div key={i} className="break-all">
            <span className="text-content-tertiary select-none">$ </span>
            {cmd.command}
          </div>
        ))}
      </div>
    </SystemAlertCard>
  );
}
