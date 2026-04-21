import { Check, Shield, Terminal, X } from 'lucide-react';
import { useEffect, useState } from 'react';
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

const APPROVAL_TIMEOUT_SECS = 60;

export function CommandApprovalCard({ approvalId, commands, onResolved }: Props) {
  const api = useApi();
  const [status, setStatus] = useState<'pending' | 'acting' | 'resolved'>('pending');
  const [secondsLeft, setSecondsLeft] = useState(APPROVAL_TIMEOUT_SECS);

  useEffect(() => {
    if (status !== 'pending') return;
    const interval = setInterval(() => {
      setSecondsLeft((prev) => {
        if (prev <= 1) {
          clearInterval(interval);
          setStatus('resolved');
          onResolved(approvalId);
          return 0;
        }
        return prev - 1;
      });
    }, 1000);
    return () => clearInterval(interval);
  }, [status, approvalId, onResolved]);

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
      trailing={<span className="text-[10px] font-mono text-content-tertiary">{secondsLeft}s</span>}
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
      {/* Countdown bar */}
      <div className="h-0.5 bg-amber-500/10 rounded-full overflow-hidden">
        <div
          className="h-full bg-amber-500/40 transition-all duration-1000 ease-linear"
          style={{ width: `${(secondsLeft / APPROVAL_TIMEOUT_SECS) * 100}%` }}
        />
      </div>

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
