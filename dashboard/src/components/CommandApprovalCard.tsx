import { useState, useEffect } from 'react';
import { Terminal, Check, Shield, X } from 'lucide-react';
import { api } from '../services/api';
import { useApiKey } from '../contexts/ApiKeyContext';

interface Props {
  approvalId: string;
  command: string;
  commandName: string;
  onResolved: (approvalId: string) => void;
}

export function CommandApprovalCard({ approvalId, command, commandName, onResolved }: Props) {
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

  return (
    <div className="bg-surface-primary/90 backdrop-blur-xl border border-edge rounded-xl shadow-lg p-4 space-y-3 animate-in fade-in slide-in-from-bottom-2 duration-300 max-w-md">
      {/* Header */}
      <div className="flex items-center gap-2">
        <Terminal size={14} className="text-amber-500" />
        <span className="text-[10px] font-black text-amber-500 uppercase tracking-[0.2em]">Command Approval</span>
        <span className="ml-auto text-[10px] font-mono text-content-muted">{secondsLeft}s</span>
      </div>

      {/* Countdown bar */}
      <div className="h-0.5 bg-edge rounded-full overflow-hidden">
        <div
          className="h-full bg-amber-500/60 transition-all duration-1000 ease-linear"
          style={{ width: `${(secondsLeft / 60) * 100}%` }}
        />
      </div>

      {/* Command display */}
      <div className="bg-[#0d1117] rounded-lg px-3 py-2 font-mono text-xs text-emerald-400 break-all">
        <span className="text-content-muted select-none">$ </span>{command}
      </div>

      {/* Action buttons */}
      <div className="flex gap-2">
        <button
          onClick={() => handle('approve')}
          disabled={status === 'acting'}
          className="flex-1 flex items-center justify-center gap-1.5 py-2 rounded-lg bg-emerald-600 text-white text-[10px] font-bold uppercase tracking-wider hover:bg-emerald-500 disabled:opacity-50 transition-all"
        >
          <Check size={12} /> Yes
        </button>
        <button
          onClick={() => handle('trust')}
          disabled={status === 'acting'}
          className="flex-1 flex items-center justify-center gap-1.5 py-2 rounded-lg bg-brand text-white text-[10px] font-bold uppercase tracking-wider hover:bg-brand/80 disabled:opacity-50 transition-all"
        >
          <Shield size={12} /> Trust &apos;{commandName}&apos;
        </button>
        <button
          onClick={() => handle('deny')}
          disabled={status === 'acting'}
          className="flex-1 flex items-center justify-center gap-1.5 py-2 rounded-lg border border-edge text-[10px] font-bold uppercase tracking-wider text-content-secondary hover:text-red-400 hover:border-red-500/30 disabled:opacity-50 transition-all"
        >
          <X size={12} /> No
        </button>
      </div>
    </div>
  );
}
