import { useState, useEffect } from 'react';
import { Terminal, Check, Shield, X } from 'lucide-react';
import { SystemAlertCard } from './SystemAlertCard';
import { api } from '../services/api';
import { useApiKey } from '../contexts/ApiKeyContext';

interface CommandEntry {
  command: string;
  command_name: string;
}

interface Props {
  approvalId: string;
  commands: CommandEntry[];
  onResolved: (approvalId: string) => void;
}

export function CommandApprovalCard({ approvalId, commands, onResolved }: Props) {
  const { apiKey } = useApiKey();
  const effectiveKey = apiKey || '';
  const [status, setStatus] = useState<'pending' | 'acting' | 'resolved'>('pending');
  const [secondsLeft, setSecondsLeft] = useState(60);

  useEffect(() => {
    if (status !== 'pending') return;
    const interval = setInterval(() => {
      setSecondsLeft(prev => {
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
      if (action === 'approve') await api.approveCommand(approvalId, effectiveKey);
      else if (action === 'trust') await api.trustCommand(approvalId, effectiveKey);
      else await api.denyCommand(approvalId, effectiveKey);
      setStatus('resolved');
      onResolved(approvalId);
    } catch {
      setStatus('pending');
    }
  };

  if (status === 'resolved') return null;

  const uniqueNames = [...new Set(commands.map(c => c.command_name))];
  const trustLabel = uniqueNames.length === 1
    ? `Trust '${uniqueNames[0]}'`
    : `Trust ${uniqueNames.length} commands`;

  return (
    <SystemAlertCard
      icon={<Terminal size={14} />}
      title={`Command Approval${commands.length > 1 ? ` (${commands.length})` : ''}`}
      trailing={<span className="text-[10px] font-mono text-content-muted">{secondsLeft}s</span>}
      footer={
        <div className="flex gap-2">
          <button
            onClick={() => handle('approve')}
            disabled={status === 'acting'}
            className="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[10px] font-bold uppercase tracking-wider text-emerald-400 hover:bg-emerald-500/10 disabled:opacity-50 transition-colors"
          >
            <Check size={12} /> Yes
          </button>
          <button
            onClick={() => handle('trust')}
            disabled={status === 'acting'}
            className="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[10px] font-bold uppercase tracking-wider text-amber-500 hover:bg-amber-500/10 disabled:opacity-50 transition-colors"
          >
            <Shield size={12} /> {trustLabel}
          </button>
          <button
            onClick={() => handle('deny')}
            disabled={status === 'acting'}
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
          style={{ width: `${(secondsLeft / 60) * 100}%` }}
        />
      </div>

      {/* Command display */}
      <div className="font-mono text-xs text-content-secondary space-y-1 whitespace-pre-line">
        {commands.map((cmd, i) => (
          <div key={i} className="break-all">
            <span className="text-content-muted select-none">$ </span>{cmd.command}
          </div>
        ))}
      </div>
    </SystemAlertCard>
  );
}
