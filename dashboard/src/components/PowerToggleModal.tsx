import React, { useState } from 'react';
import { Power, Lock } from 'lucide-react';
import { AgentMetadata } from '../types';
import { api } from '../services/api';

interface Props {
  agent: AgentMetadata;
  onClose: () => void;
  onSuccess: () => void;
}

export function PowerToggleModal({ agent, onClose, onSuccess }: Props) {
  const [password, setPassword] = useState('');
  const [error, setError] = useState('');
  const [isLoading, setIsLoading] = useState(false);

  const handleConfirm = async () => {
    setIsLoading(true);
    setError('');
    try {
      await api.toggleAgentPower(agent.id, !agent.enabled, password);
      onClose();
      onSuccess();
    } catch (err: any) {
      setError(err.message || 'Failed to toggle power');
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/20 backdrop-blur-sm">
      <div className="bg-white rounded-2xl shadow-2xl p-6 w-80 space-y-4 animate-in fade-in zoom-in-95 duration-200">
        <div className="flex items-center gap-3">
          <div className={`p-2 rounded-lg ${agent.enabled ? 'bg-red-50 text-red-500' : 'bg-emerald-50 text-emerald-500'}`}>
            <Power size={18} />
          </div>
          <div>
            <h3 className="text-sm font-bold text-slate-800">
              {agent.enabled ? 'Power Off' : 'Power On'} {agent.name}
            </h3>
            <p className="text-[10px] text-slate-400">Enter power password to continue</p>
          </div>
        </div>
        <div className="relative">
          <Lock size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-slate-300" />
          <input
            type="password"
            value={password}
            onChange={e => setPassword(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && password && handleConfirm()}
            className="w-full pl-9 pr-3 py-2.5 rounded-xl border border-slate-200 text-sm focus:outline-none focus:border-[#2e4de6]"
            placeholder="Password"
            autoFocus
          />
        </div>
        {error && (
          <p className="text-[10px] text-red-500 font-medium">{error}</p>
        )}
        <div className="flex gap-2">
          <button
            onClick={onClose}
            className="flex-1 py-2 rounded-xl border border-slate-200 text-xs font-bold text-slate-500 hover:bg-slate-50 transition-all"
            disabled={isLoading}
          >
            Cancel
          </button>
          <button
            onClick={handleConfirm}
            disabled={!password || isLoading}
            className={`flex-1 py-2 rounded-xl text-white text-xs font-bold transition-all disabled:opacity-50 ${
              agent.enabled ? 'bg-red-500 hover:bg-red-600' : 'bg-emerald-500 hover:bg-emerald-600'
            }`}
          >
            {isLoading ? 'Processing...' : 'Confirm'}
          </button>
        </div>
      </div>
    </div>
  );
}
