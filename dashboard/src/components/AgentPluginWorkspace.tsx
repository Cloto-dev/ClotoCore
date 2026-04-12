import { Activity, ArrowLeft, Save } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../hooks/useApi';
import { useMcpServers } from '../hooks/useMcpServers';
import { AgentIcon, agentColor } from '../lib/agentIdentity';
import { extractVrmThumbnail } from '../lib/vrmThumbnail';
import type { AgentMetadata } from '../types';
import { AvatarSection } from './AvatarSection';
import { ProfileSection } from './ProfileSection';
import { ServerAccessSection } from './ServerAccessSection';
import { AlertCard } from './ui/AlertCard';
import { VrmThumbnailDialog } from './VrmThumbnailDialog';

interface Props {
  agent: AgentMetadata;
  onBack: () => void;
}

const DEFAULT_AGENT_ID = 'agent.cloto_default';

export function AgentPluginWorkspace({ agent, onBack }: Props) {
  const { t } = useTranslation('agents');
  const { t: tc } = useTranslation('common');
  const api = useApi();
  const isDefault = agent.id === DEFAULT_AGENT_ID;
  const { servers } = useMcpServers();

  const [grantedIds, setGrantedIds] = useState<Set<string>>(new Set());
  const [isSaving, setIsSaving] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [saveError, setSaveError] = useState('');

  // Profile state
  const [agentName, setAgentName] = useState(agent.name);
  const [agentDescription, setAgentDescription] = useState(agent.description);

  // Avatar state (deferred — only persisted on Save)
  const [avatarKey, _setAvatarKey] = useState(0);
  const [hasAvatar, setHasAvatar] = useState(agent.metadata?.has_avatar === 'true');
  const [avatarDescription, setAvatarDescription] = useState(agent.metadata?.avatar_description || '');
  const [pendingAvatarFile, setPendingAvatarFile] = useState<File | null>(null);
  const [pendingAvatarDelete, setPendingAvatarDelete] = useState(false);
  const [avatarPreviewUrl, setAvatarPreviewUrl] = useState<string | null>(null);

  // VRM state (deferred — only persisted on Save, same as avatar)
  const [hasVrm, setHasVrm] = useState(agent.metadata?.has_vrm === 'true');
  const [pendingVrmFile, setPendingVrmFile] = useState<File | null>(null);
  const [pendingVrmDelete, setPendingVrmDelete] = useState(false);
  const [vrmThumbnailFile, setVrmThumbnailFile] = useState<File | null>(null);
  const [vrmThumbnailUrl, setVrmThumbnailUrl] = useState<string | null>(null);
  const [showVrmThumbnailDialog, setShowVrmThumbnailDialog] = useState(false);

  // Load current access entries for this agent
  useEffect(() => {
    api
      .getAgentAccess(agent.id)
      .then((data) => {
        const granted = new Set(
          data.entries
            .filter((e) => e.entry_type === 'server_grant' && e.permission === 'allow')
            .map((e) => e.server_id),
        );
        setGrantedIds(granted);
      })
      .catch((e) => {
        if (import.meta.env.DEV) console.error('Failed to load agent access:', e);
      })
      .finally(() => setIsLoading(false));
  }, [agent.id, api.getAgentAccess]);

  const grantServer = (serverId: string) => {
    setGrantedIds((prev) => new Set([...prev, serverId]));
  };

  const revokeServer = (serverId: string) => {
    setGrantedIds((prev) => {
      const next = new Set(prev);
      next.delete(serverId);
      return next;
    });
  };

  const applyPreset = (presetServerIds: string[]) => {
    setGrantedIds((prev) => {
      // Keep existing mind.* engines, replace everything else with preset
      const engines = [...prev].filter((id) => id.startsWith('mind.'));
      return new Set([...engines, ...presetServerIds]);
    });
  };

  const handleSave = async () => {
    setIsSaving(true);
    setSaveError('');

    try {
      // Bulk replace this agent's server_grant entries in a single request.
      // Avoids the 2N REST-call pattern that previously tripped the rate limiter
      // when users toggled many servers at once. Backend preserves tool_grant
      // and capability entries.
      await api.putAgentMcpAccess(agent.id, [...grantedIds]);

      // Derive default_engine_id and preferred_memory from granted servers
      const grantedServers = servers.filter((s) => grantedIds.has(s.id));
      const engineServer = grantedServers.find((s) => s.id.startsWith('mind.'));
      const memoryServer = grantedServers.find((s) => s.id.startsWith('memory.'));

      const metadata: Record<string, string> = { ...agent.metadata };
      // Remove fields managed by dedicated APIs (avatar, VRM, password).
      // updateAgent uses COALESCE(?, metadata) which does FULL REPLACEMENT,
      // so these fields must not be present to avoid conflicts.
      delete metadata.has_avatar;
      delete metadata.avatar_path;
      delete metadata.avatar_description;
      delete metadata.has_power_password;
      delete metadata.has_vrm;
      delete metadata.vrm_path;
      if (memoryServer) {
        metadata.preferred_memory = memoryServer.id;
      } else {
        delete metadata.preferred_memory;
      }

      // Step 1: updateAgent FIRST (full metadata replacement via COALESCE)
      await api.updateAgent(agent.id, {
        name: agentName !== agent.name ? agentName : undefined,
        description: agentDescription !== agent.description ? agentDescription : undefined,
        default_engine_id: engineServer?.id,
        metadata,
      });

      // Step 2: Avatar operations AFTER updateAgent — these use json_set/json_remove
      // which do partial updates, so avatar_path survives as the final state.
      if (pendingAvatarDelete && !pendingAvatarFile) {
        await api.deleteAvatar(agent.id);
      }
      if (pendingAvatarFile) {
        await api.uploadAvatar(agent.id, pendingAvatarFile);
      }

      // Step 3: VRM operations AFTER updateAgent (same deferred pattern as avatar)
      if (pendingVrmDelete && !pendingVrmFile) {
        await api.deleteVrm(agent.id);
      }
      if (pendingVrmFile) {
        await api.uploadVrm(agent.id, pendingVrmFile);
      }

      // Clean up preview URL
      if (avatarPreviewUrl) URL.revokeObjectURL(avatarPreviewUrl);

      onBack();
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Failed to save configuration';
      setSaveError(message);
    } finally {
      setIsSaving(false);
    }
  };

  const handleAvatarUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    if (file.size > 5 * 1024 * 1024) {
      setSaveError(t('plugin_workspace.avatar_too_large'));
      return;
    }
    // Store file locally — upload deferred to Save
    if (avatarPreviewUrl) URL.revokeObjectURL(avatarPreviewUrl);
    setPendingAvatarFile(file);
    setPendingAvatarDelete(false);
    setAvatarPreviewUrl(URL.createObjectURL(file));
    setHasAvatar(true);
    setAvatarDescription('');
    e.target.value = '';
  };

  const handleAvatarDelete = () => {
    // Mark for deletion locally — actual delete deferred to Save
    if (avatarPreviewUrl) URL.revokeObjectURL(avatarPreviewUrl);
    setPendingAvatarFile(null);
    setPendingAvatarDelete(true);
    setAvatarPreviewUrl(null);
    setHasAvatar(false);
    setAvatarDescription('');
  };

  // VRM handlers — deferred to Save (same pattern as avatar)
  const handleVrmUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    if (file.size > 50 * 1024 * 1024) {
      setSaveError('VRM file too large (max 50MB)');
      return;
    }
    setPendingVrmFile(file);
    setPendingVrmDelete(false);
    setHasVrm(true);
    e.target.value = '';

    // Extract thumbnail and offer as avatar (unless user opted out)
    if (sessionStorage.getItem('cloto-vrm-thumbnail-skip') === '1') return;
    try {
      const thumbnail = await extractVrmThumbnail(file);
      if (thumbnail) {
        const url = URL.createObjectURL(thumbnail);
        setVrmThumbnailFile(thumbnail);
        setVrmThumbnailUrl(url);
        setShowVrmThumbnailDialog(true);
      }
    } catch {
      // Thumbnail extraction failure is non-fatal
    }
  };

  const handleVrmThumbnailApply = () => {
    if (avatarPreviewUrl) URL.revokeObjectURL(avatarPreviewUrl);
    setPendingAvatarFile(vrmThumbnailFile);
    setPendingAvatarDelete(false);
    setAvatarPreviewUrl(vrmThumbnailUrl);
    setHasAvatar(true);
    setAvatarDescription('');
    setShowVrmThumbnailDialog(false);
  };

  const handleVrmThumbnailSkip = () => {
    if (vrmThumbnailUrl) URL.revokeObjectURL(vrmThumbnailUrl);
    setVrmThumbnailFile(null);
    setVrmThumbnailUrl(null);
    setShowVrmThumbnailDialog(false);
  };

  const handleVrmDelete = () => {
    setPendingVrmFile(null);
    setPendingVrmDelete(true);
    setHasVrm(false);
  };

  const grantedServers = servers.filter((s) => grantedIds.has(s.id));
  const availableServers = servers.filter((s) => !grantedIds.has(s.id));

  return (
    <div className="flex flex-col h-full overflow-hidden animate-in fade-in duration-500">
      {/* Header */}
      <header className="p-6 flex items-center justify-between border-b border-edge">
        <div className="flex items-center gap-4">
          <button
            onClick={onBack}
            aria-label={tc('back')}
            className="p-2.5 rounded-full bg-glass-subtle backdrop-blur-sm border border-edge hover:border-brand hover:text-brand transition-all"
          >
            <ArrowLeft size={18} />
          </button>
          <div
            className="w-10 h-10 rounded-md flex items-center justify-center shadow-sm text-white overflow-hidden"
            style={{ backgroundColor: agentColor(agent) }}
          >
            <AgentIcon agent={agent} size={40} />
          </div>
          <div>
            <h1 className="text-xl font-black tracking-tighter text-content-primary uppercase">
              {agent.name} · {t('plugin_workspace.mcp_access')}
            </h1>
            <p className="text-[10px] text-content-tertiary font-mono uppercase tracking-[0.2em]">
              {t('plugin_workspace.server_access_control')}
            </p>
          </div>
        </div>
        <div className="bg-glass-subtle backdrop-blur-sm px-4 py-2 rounded-md flex items-center gap-3 shadow-sm border border-edge">
          <span className="text-[9px] uppercase font-bold text-content-tertiary tracking-widest">
            {t('plugin_workspace.granted_count', { count: grantedIds.size })}
          </span>
        </div>
      </header>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-6 md:p-8 space-y-6 no-scrollbar">
        {isLoading ? (
          <div className="py-12 text-center text-content-tertiary font-mono text-xs animate-pulse">{tc('loading')}</div>
        ) : (
          <>
            {/* Avatar (protected for default agent) */}
            {!isDefault && (
              <AvatarSection
                agent={agent}
                hasAvatar={hasAvatar}
                hasVrm={hasVrm}
                avatarKey={avatarKey}
                avatarDescription={avatarDescription}
                previewUrl={avatarPreviewUrl}
                onUpload={handleAvatarUpload}
                onDelete={handleAvatarDelete}
                onVrmUpload={handleVrmUpload}
                onVrmDelete={handleVrmDelete}
              />
            )}

            {/* Profile (protected for default agent) */}
            {!isDefault && (
              <ProfileSection
                name={agentName}
                description={agentDescription}
                onNameChange={setAgentName}
                onDescriptionChange={setAgentDescription}
              />
            )}

            {/* Server Access Control */}
            <ServerAccessSection
              grantedServers={grantedServers}
              availableServers={availableServers}
              agentColorHex={agentColor(agent)}
              grantedIds={grantedIds}
              onGrant={grantServer}
              onRevoke={revokeServer}
              onApplyPreset={applyPreset}
            />
          </>
        )}
      </div>

      {/* VRM Thumbnail Dialog */}
      <VrmThumbnailDialog
        open={showVrmThumbnailDialog}
        thumbnailUrl={vrmThumbnailUrl ?? ''}
        onApply={handleVrmThumbnailApply}
        onSkip={handleVrmThumbnailSkip}
      />

      {/* Footer */}
      <div className="p-4 border-t border-edge flex items-center justify-between">
        {saveError && <AlertCard>{saveError}</AlertCard>}
        <div className="flex-1" />
        <div className="flex gap-2">
          <button
            onClick={onBack}
            aria-label={tc('cancel')}
            className="px-4 py-2 rounded-lg border border-edge text-xs font-bold text-content-secondary hover:bg-surface-secondary transition-all"
          >
            {tc('cancel')}
          </button>
          <button
            onClick={handleSave}
            disabled={isSaving || isLoading}
            aria-label={tc('save')}
            className="flex items-center gap-1.5 px-6 py-2 rounded-lg bg-brand text-white text-xs font-bold shadow-sm hover:shadow-md transition-all disabled:opacity-50"
          >
            {isSaving ? <Activity size={14} className="animate-spin" /> : <Save size={14} />}
            {tc('save')}
          </button>
        </div>
      </div>
    </div>
  );
}
