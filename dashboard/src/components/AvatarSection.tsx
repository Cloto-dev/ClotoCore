import { Box, Camera, Upload, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { AgentIcon } from '../lib/agentIdentity';
import { api } from '../services/api';
import type { AgentMetadata } from '../types';

interface Props {
  agent: AgentMetadata;
  hasAvatar: boolean;
  hasVrm: boolean;
  avatarKey: number;
  avatarDescription: string;
  previewUrl: string | null;
  onUpload: (e: React.ChangeEvent<HTMLInputElement>) => void;
  onDelete: () => void;
  onVrmUpload: (e: React.ChangeEvent<HTMLInputElement>) => void;
  onVrmDelete: () => void;
}

export function AvatarSection({
  agent,
  hasAvatar,
  hasVrm,
  avatarKey,
  avatarDescription,
  previewUrl,
  onUpload,
  onDelete,
  onVrmUpload,
  onVrmDelete,
}: Props) {
  const { t } = useTranslation('agents');
  const displayUrl = previewUrl ?? (hasAvatar ? `${api.getAvatarUrl(agent.id)}?v=${avatarKey}` : null);

  return (
    <section>
      <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
        <Camera className="text-brand" size={16} />
        <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">
          {t('plugin_workspace.avatar')}
        </h2>
      </div>
      <div className="space-y-3">
        <div className="flex items-center gap-4">
          <div className="w-24 h-24 rounded-lg border border-edge overflow-hidden flex items-center justify-center bg-glass-strong shrink-0">
            {displayUrl ? (
              <img src={displayUrl} alt="Avatar" className="w-full h-full object-cover" />
            ) : (
              <AgentIcon agent={agent} size={48} />
            )}
          </div>
          <div className="flex flex-col gap-2">
            <label className="cursor-pointer inline-flex items-center gap-1.5 px-4 py-2 rounded-lg border border-edge text-xs font-bold text-content-secondary hover:text-brand hover:border-brand transition-all">
              <Upload size={14} />
              {t('plugin_workspace.avatar_upload')}
              <input
                type="file"
                accept="image/png,image/jpeg,image/gif,image/webp"
                className="hidden"
                onChange={onUpload}
              />
            </label>
            {hasAvatar && (
              <button
                onClick={onDelete}
                className="inline-flex items-center gap-1.5 px-4 py-2 rounded-lg border border-red-500/20 bg-red-500/5 text-xs font-bold text-red-400/70 hover:text-red-500 hover:border-red-500/40 hover:bg-red-500/10 transition-all"
              >
                <X size={14} /> {t('plugin_workspace.avatar_remove')}
              </button>
            )}
          </div>
        </div>
        {avatarDescription && (
          <div className="text-[11px] text-content-tertiary font-mono bg-glass rounded-lg p-3 border border-edge leading-relaxed">
            <span className="text-content-tertiary font-bold">{t('plugin_workspace.avatar_vision')} </span>
            {avatarDescription}
          </div>
        )}

        {/* VRM 3D Model section */}
        <div className="pt-3 border-t border-edge-subtle">
          <div className="flex items-center gap-2 mb-2">
            <Box size={14} className="text-brand" />
            <span className="text-[11px] font-bold text-content-secondary uppercase tracking-widest">VRM 3D Model</span>
          </div>
          <div className="flex items-center gap-3">
            {hasVrm ? (
              <>
                <span className="text-[10px] font-mono text-brand/80 bg-brand/5 px-2 py-1 rounded border border-brand/20">
                  VRM Loaded
                </span>
                <button
                  onClick={onVrmDelete}
                  className="inline-flex items-center gap-1 px-3 py-1.5 rounded-lg border border-red-500/20 bg-red-500/5 text-[10px] font-bold text-red-400/70 hover:text-red-500 hover:border-red-500/40 hover:bg-red-500/10 transition-all"
                >
                  <X size={12} /> Remove
                </button>
              </>
            ) : (
              <label className="cursor-pointer inline-flex items-center gap-1.5 px-4 py-2 rounded-lg border border-edge text-xs font-bold text-content-secondary hover:text-brand hover:border-brand transition-all">
                <Upload size={14} />
                Upload VRM
                <input type="file" accept=".vrm,model/gltf-binary" className="hidden" onChange={onVrmUpload} />
              </label>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
