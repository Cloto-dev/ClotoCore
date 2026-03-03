import { Camera, Upload, X } from 'lucide-react';
import { AgentMetadata } from '../types';
import { api } from '../services/api';
import { AgentIcon } from '../lib/agentIdentity';

interface Props {
  agent: AgentMetadata;
  hasAvatar: boolean;
  avatarKey: number;
  avatarDescription: string;
  isUploading: boolean;
  onUpload: (e: React.ChangeEvent<HTMLInputElement>) => void;
  onDelete: () => void;
}

export function AvatarSection({ agent, hasAvatar, avatarKey, avatarDescription, isUploading, onUpload, onDelete }: Props) {
  return (
    <section>
      <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
        <Camera className="text-brand" size={16} />
        <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">Avatar</h2>
      </div>
      <div className="space-y-3">
        <div className="flex items-center gap-4">
          <div className="w-24 h-24 rounded-lg border border-edge overflow-hidden flex items-center justify-center bg-glass-strong shrink-0">
            {hasAvatar ? (
              <img
                src={`${api.getAvatarUrl(agent.id)}?v=${avatarKey}`}
                alt="Avatar"
                className="w-full h-full object-cover"
              />
            ) : (
              <AgentIcon agent={agent} size={48} />
            )}
          </div>
          <div className="flex flex-col gap-2">
            <label className={`cursor-pointer inline-flex items-center gap-1.5 px-4 py-2 rounded-lg border border-edge text-xs font-bold text-content-secondary hover:text-brand hover:border-brand transition-all ${isUploading ? 'opacity-50 pointer-events-none' : ''}`}>
              <Upload size={14} />
              {isUploading ? 'Analyzing...' : 'Upload'}
              <input
                type="file"
                accept="image/png,image/jpeg,image/gif,image/webp"
                className="hidden"
                onChange={onUpload}
                disabled={isUploading}
              />
            </label>
            {hasAvatar && (
              <button
                onClick={onDelete}
                className="inline-flex items-center gap-1.5 px-4 py-2 rounded-lg border border-red-500/20 bg-red-500/5 text-xs font-bold text-red-400/70 hover:text-red-500 hover:border-red-500/40 hover:bg-red-500/10 transition-all"
              >
                <X size={14} /> Remove
              </button>
            )}
          </div>
        </div>
        {avatarDescription && (
          <div className="text-[11px] text-content-tertiary font-mono bg-glass rounded-lg p-3 border border-edge leading-relaxed">
            <span className="text-content-muted font-bold">Vision: </span>{avatarDescription}
          </div>
        )}
      </div>
    </section>
  );
}
