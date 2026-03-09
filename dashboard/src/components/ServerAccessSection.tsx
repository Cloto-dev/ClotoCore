import { Server, Plus, X, Wifi, WifiOff, AlertTriangle, Layers } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { McpServerInfo } from '../types';
import { displayServerId } from '../lib/format';
import { SERVER_PRESETS, detectPreset } from '../lib/presets';

const StatusIcon = ({ status }: { status: McpServerInfo['status'] }) => {
  switch (status) {
    case 'Connected': return <Wifi size={12} className="text-emerald-500" />;
    case 'Disconnected': return <WifiOff size={12} className="text-content-muted" />;
    case 'Error': return <AlertTriangle size={12} className="text-red-500" />;
  }
};

const StatusBadge = ({ status, t }: { status: McpServerInfo['status']; t: (key: string) => string }) => {
  const labelKey = status === 'Connected' ? 'plugin_workspace.status_connected'
    : status === 'Error' ? 'plugin_workspace.status_error'
    : 'plugin_workspace.status_disconnected';
  return (
    <span className={`text-[9px] font-bold uppercase tracking-wider px-2 py-0.5 rounded ${
      status === 'Connected' ? 'bg-emerald-500/10 text-emerald-500' :
      status === 'Error' ? 'bg-red-500/10 text-red-500' :
      'bg-surface-secondary text-content-tertiary'
    }`}>
      {t(labelKey)}
    </span>
  );
};

interface Props {
  grantedServers: McpServerInfo[];
  availableServers: McpServerInfo[];
  agentColorHex: string;
  grantedIds: Set<string>;
  onGrant: (serverId: string) => void;
  onRevoke: (serverId: string) => void;
  onApplyPreset: (serverIds: string[]) => void;
}

export function ServerAccessSection({ grantedServers, availableServers, agentColorHex, grantedIds, onGrant, onRevoke, onApplyPreset }: Props) {
  const { t } = useTranslation('agents');
  const activePreset = detectPreset(grantedIds);

  return (
    <>
      {/* Preset Selector */}
      <section>
        <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
          <Layers className="text-brand" size={16} />
          <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">{t('plugin_workspace.preset')}</h2>
        </div>
        <div className="flex flex-wrap gap-2">
          {SERVER_PRESETS.map(preset => {
            const isActive = activePreset === preset.id;
            return (
              <button
                key={preset.id}
                onClick={() => onApplyPreset(preset.servers)}
                className={`px-3 py-1.5 rounded-lg text-[11px] font-bold uppercase tracking-wider border transition-all ${
                  isActive
                    ? 'border-brand bg-brand/10 text-brand'
                    : 'border-edge bg-glass text-content-secondary hover:border-brand/40 hover:text-brand'
                }`}
              >
                {t(`plugin_workspace.preset_${preset.id}`)}
                <span className="ml-1.5 text-[9px] font-mono text-content-tertiary">
                  {preset.servers.length}
                </span>
              </button>
            );
          })}
        </div>
      </section>

      {/* Granted Servers */}
      <section>
        <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
          <Server className="text-brand" size={16} />
          <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">{t('plugin_workspace.granted_servers')}</h2>
        </div>
        {grantedServers.length === 0 ? (
          <div className="py-8 text-center text-content-tertiary bg-glass rounded-lg border border-edge border-dashed font-mono text-xs">
            {t('plugin_workspace.no_servers_granted')}
          </div>
        ) : (
          <div className="space-y-2">
            {grantedServers.map(server => (
              <div
                key={server.id}
                className="bg-glass-strong backdrop-blur-sm px-4 py-3 rounded-lg border border-edge hover:border-red-500/30 transition-all flex items-center gap-3 group cursor-pointer"
                onClick={() => onRevoke(server.id)}
              >
                <div className="p-1.5 rounded-md" style={{ backgroundColor: `${agentColorHex}15`, color: agentColorHex }}>
                  <Server size={16} />
                </div>
                <div className="flex-1 min-w-0">
                  <span className="text-sm font-bold text-content-primary">{displayServerId(server.id)}</span>
                  <span className="text-xs text-content-tertiary ml-2 font-mono">{t('plugin_workspace.tools_count', { count: server.tools.length })}</span>
                </div>
                <StatusIcon status={server.status} />
                <StatusBadge status={server.status} t={t} />
                <span className="p-1.5 rounded text-content-muted group-hover:text-red-500 transition-all opacity-0 group-hover:opacity-100">
                  <X size={14} />
                </span>
              </div>
            ))}
          </div>
        )}
      </section>

      {/* Available Servers */}
      {availableServers.length > 0 && (
        <section>
          <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
            <Plus className="text-brand" size={16} />
            <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">{t('plugin_workspace.available')}</h2>
          </div>
          <div className="space-y-2">
            {availableServers.map(server => (
              <div
                key={server.id}
                className="bg-glass backdrop-blur-sm px-4 py-3 rounded-lg border border-edge hover:border-brand/50 transition-all flex items-center gap-3 group cursor-pointer"
                onClick={() => onGrant(server.id)}
              >
                <div className="p-1.5 rounded-md text-content-muted group-hover:text-brand transition-colors">
                  <Server size={16} />
                </div>
                <div className="flex-1 min-w-0">
                  <span className="text-sm font-medium text-content-secondary">{displayServerId(server.id)}</span>
                  <span className="text-xs text-content-tertiary ml-2 font-mono">{t('plugin_workspace.tools_count', { count: server.tools.length })}</span>
                </div>
                <StatusIcon status={server.status} />
                <StatusBadge status={server.status} t={t} />
                <span className="inline-flex items-center gap-1 px-2 py-1 rounded text-[10px] font-bold text-brand opacity-0 group-hover:opacity-100 transition-all">
                  <Plus size={10} /> {t('plugin_workspace.grant')}
                </span>
              </div>
            ))}
          </div>
        </section>
      )}
    </>
  );
}
