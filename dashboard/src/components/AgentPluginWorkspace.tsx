import { useState, useEffect, useRef } from 'react';
import { ArrowLeft, Save, Activity } from 'lucide-react';
import { AgentMetadata, AccessControlEntry } from '../types';
import { api } from '../services/api';
import { AgentIcon, agentColor } from '../lib/agentIdentity';
import { useApiKey } from '../contexts/ApiKeyContext';
import { useMcpServers } from '../hooks/useMcpServers';
import { AvatarSection } from './AvatarSection';
import { ProfileSection } from './ProfileSection';
import { ServerAccessSection } from './ServerAccessSection';

interface Props {
  agent: AgentMetadata;
  onBack: () => void;
}

const DEFAULT_AGENT_ID = 'agent.cloto_default';

export function AgentPluginWorkspace({ agent, onBack }: Props) {
  const { apiKey } = useApiKey();
  // Allow empty apiKey — debug backend skips auth when CLOTO_API_KEY is unset
  const effectiveKey = apiKey || '';
  const isDefault = agent.id === DEFAULT_AGENT_ID;
  const { servers } = useMcpServers(effectiveKey);

  const [grantedIds, setGrantedIds] = useState<Set<string>>(new Set());
  const initialGrantedRef = useRef<Set<string>>(new Set());
  const [isSaving, setIsSaving] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [saveError, setSaveError] = useState('');

  // Profile state
  const [agentName, setAgentName] = useState(agent.name);
  const [agentDescription, setAgentDescription] = useState(agent.description);

  // Avatar state
  const [avatarKey, setAvatarKey] = useState(0);
  const [hasAvatar, setHasAvatar] = useState(agent.metadata?.has_avatar === 'true');
  const [avatarDescription, setAvatarDescription] = useState(agent.metadata?.avatar_description || '');
  const [isUploadingAvatar, setIsUploadingAvatar] = useState(false);

  // Load current access entries for this agent
  useEffect(() => {
    api.getAgentAccess(agent.id)
      .then(data => {
        const granted = new Set(
          data.entries
            .filter(e => e.entry_type === 'server_grant' && e.permission === 'allow')
            .map(e => e.server_id)
        );
        setGrantedIds(granted);
        initialGrantedRef.current = new Set(granted);
      })
      .catch(e => {
        console.error('Failed to load agent access:', e);
      })
      .finally(() => setIsLoading(false));
  }, [agent.id]);

  const grantServer = (serverId: string) => {
    setGrantedIds(prev => new Set([...prev, serverId]));
  };

  const revokeServer = (serverId: string) => {
    setGrantedIds(prev => {
      const next = new Set(prev);
      next.delete(serverId);
      return next;
    });
  };

  const handleSave = async () => {
    setIsSaving(true);
    setSaveError('');

    try {
      const initial = initialGrantedRef.current;
      const added = [...grantedIds].filter(id => !initial.has(id));
      const removed = [...initial].filter(id => !grantedIds.has(id));

      const now = new Date().toISOString();

      // Process added servers
      for (const serverId of added) {
        const tree = await api.getMcpServerAccess(serverId, effectiveKey);
        const existing = tree.entries.filter(
          e => !(e.agent_id === agent.id && e.entry_type === 'server_grant')
        );
        const newEntry: AccessControlEntry = {
          entry_type: 'server_grant',
          agent_id: agent.id,
          server_id: serverId,
          permission: 'allow',
          granted_by: 'admin',
          granted_at: now,
        };
        await api.putMcpServerAccess(serverId, [...existing, newEntry], effectiveKey);
      }

      // Process removed servers
      for (const serverId of removed) {
        const tree = await api.getMcpServerAccess(serverId, effectiveKey);
        const filtered = tree.entries.filter(
          e => !(e.agent_id === agent.id && e.entry_type === 'server_grant')
        );
        await api.putMcpServerAccess(serverId, filtered, effectiveKey);
      }

      // Derive default_engine_id and preferred_memory from granted servers
      const grantedServers = servers.filter(s => grantedIds.has(s.id));
      const engineServer = grantedServers.find(s => s.id.startsWith('mind.'));
      const memoryServer = grantedServers.find(s => s.id.startsWith('memory.'));

      const metadata: Record<string, string> = { ...agent.metadata };
      // Remove backend-injected avatar fields (managed by avatar API, not metadata column)
      delete metadata.has_avatar;
      delete metadata.avatar_description;
      delete metadata.has_power_password;
      if (memoryServer) {
        metadata.preferred_memory = memoryServer.id;
      } else {
        delete metadata.preferred_memory;
      }

      await api.updateAgent(
        agent.id,
        {
          name: agentName !== agent.name ? agentName : undefined,
          description: agentDescription !== agent.description ? agentDescription : undefined,
          default_engine_id: engineServer?.id,
          metadata,
        },
        effectiveKey,
      );

      onBack();
    } catch (err: any) {
      setSaveError(err?.message || 'Failed to save configuration');
    } finally {
      setIsSaving(false);
    }
  };

  const handleAvatarUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    if (file.size > 5 * 1024 * 1024) {
      setSaveError('Avatar must be under 5MB');
      return;
    }
    setIsUploadingAvatar(true);
    setSaveError('');
    try {
      const result = await api.uploadAvatar(agent.id, file, effectiveKey);
      setAvatarKey(prev => prev + 1);
      setHasAvatar(true);
      setAvatarDescription(result.avatar_description || '');
    } catch (err: any) {
      setSaveError(err?.message || 'Failed to upload avatar');
    } finally {
      setIsUploadingAvatar(false);
      e.target.value = '';
    }
  };

  const handleAvatarDelete = async () => {
    setSaveError('');
    try {
      await api.deleteAvatar(agent.id, effectiveKey);
      setAvatarKey(prev => prev + 1);
      setHasAvatar(false);
      setAvatarDescription('');
    } catch (err: any) {
      setSaveError(err?.message || 'Failed to delete avatar');
    }
  };

  const grantedServers = servers.filter(s => grantedIds.has(s.id));
  const availableServers = servers.filter(s => !grantedIds.has(s.id));

  return (
    <div className="flex flex-col h-full overflow-hidden animate-in fade-in duration-500">
      {/* Header */}
      <header className="p-6 flex items-center justify-between border-b border-edge">
        <div className="flex items-center gap-4">
          <button
            onClick={onBack}
            className="p-2.5 rounded-full bg-glass-subtle backdrop-blur-sm border border-edge hover:border-brand hover:text-brand transition-all"
          >
            <ArrowLeft size={18} />
          </button>
          <div className="w-10 h-10 rounded-md flex items-center justify-center shadow-sm text-white overflow-hidden" style={{ backgroundColor: agentColor(agent) }}>
            <AgentIcon agent={agent} size={40} />
          </div>
          <div>
            <h1 className="text-xl font-black tracking-tighter text-content-primary uppercase">{agent.name} · MCP Access</h1>
            <p className="text-[10px] text-content-tertiary font-mono uppercase tracking-[0.2em]">Server Access Control</p>
          </div>
        </div>
        <div className="bg-glass-subtle backdrop-blur-sm px-4 py-2 rounded-md flex items-center gap-3 shadow-sm border border-edge">
          <span className="text-[9px] uppercase font-bold text-content-tertiary tracking-widest">{grantedIds.size} granted</span>
        </div>
      </header>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-6 md:p-8 space-y-6 no-scrollbar">
        {isLoading ? (
          <div className="py-12 text-center text-content-muted font-mono text-xs animate-pulse">Loading...</div>
        ) : (
          <>
            {/* Avatar (protected for default agent) */}
            {!isDefault && (
              <AvatarSection
                agent={agent}
                hasAvatar={hasAvatar}
                avatarKey={avatarKey}
                avatarDescription={avatarDescription}
                isUploading={isUploadingAvatar}
                onUpload={handleAvatarUpload}
                onDelete={handleAvatarDelete}
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
              onGrant={grantServer}
              onRevoke={revokeServer}
            />
          </>
        )}
      </div>

      {/* Footer */}
      <div className="p-4 border-t border-edge flex items-center justify-between">
        {saveError && <span className="text-[10px] text-red-400">{saveError}</span>}
        <div className="flex-1" />
        <div className="flex gap-2">
          <button
            onClick={onBack}
            className="px-4 py-2 rounded-lg border border-edge text-xs font-bold text-content-secondary hover:bg-surface-secondary transition-all"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={isSaving || isLoading}
            className="flex items-center gap-1.5 px-6 py-2 rounded-lg bg-brand text-white text-xs font-bold shadow-sm hover:shadow-md transition-all disabled:opacity-50"
          >
            {isSaving ? <Activity size={14} className="animate-spin" /> : <Save size={14} />}
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
