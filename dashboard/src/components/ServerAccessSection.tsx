import { Layers, Plus, Server, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { displayServerId } from '../lib/format';
import { detectPreset, SERVER_PRESETS } from '../lib/presets';
import type { McpServerInfo } from '../types';
import { StatusDot, type StatusDotStatus } from './ui/StatusDot';

function mcpStatusToDot(server: McpServerInfo): StatusDotStatus {
  if (server.status === 'Connected') return 'connected';
  if (server.status === 'Connecting' || server.status === 'Restarting' || server.status === 'Registered')
    return 'connecting';
  if (server.status === 'Error' && server.has_unresolved_env) return 'degraded';
  if (server.status === 'Error') return 'error';
  return 'offline';
}

function isMgpServer(server: McpServerInfo): boolean {
  return (
    (server.mgp_supported || server.id.startsWith('io.') || server.id.startsWith('output.')) &&
    server.status === 'Connected'
  );
}

function serverStatusLabel(server: McpServerInfo, t: (key: string) => string): string {
  if (server.status === 'Connected') return t('plugin_workspace.status_connected');
  if (server.status === 'Error') return t('plugin_workspace.status_error');
  return t('plugin_workspace.status_disconnected');
}

interface Props {
  grantedServers: McpServerInfo[];
  availableServers: McpServerInfo[];
  agentColorHex: string;
  grantedIds: Set<string>;
  onGrant: (serverId: string) => void;
  onRevoke: (serverId: string) => void;
  onApplyPreset: (serverIds: string[]) => void;
}

export function ServerAccessSection({
  grantedServers,
  availableServers,
  agentColorHex,
  grantedIds,
  onGrant,
  onRevoke,
  onApplyPreset,
}: Props) {
  const { t } = useTranslation('agents');
  const activePreset = detectPreset(grantedIds);

  return (
    <>
      {/* Preset Selector */}
      <section>
        <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
          <Layers className="text-brand" size={16} />
          <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">
            {t('plugin_workspace.preset')}
          </h2>
        </div>
        <div className="flex flex-wrap gap-2">
          {SERVER_PRESETS.map((preset) => {
            const isActive = activePreset === preset.id;
            return (
              <button
                key={preset.id}
                onClick={() => onApplyPreset(preset.servers)}
                aria-label={t(`plugin_workspace.preset_${preset.id}`)}
                className={`px-3 py-1.5 rounded-lg text-[11px] font-bold uppercase tracking-wider border transition-all ${
                  isActive
                    ? 'border-brand bg-brand/10 text-brand'
                    : 'border-edge bg-glass text-content-secondary hover:border-brand hover:text-brand'
                }`}
              >
                {t(`plugin_workspace.preset_${preset.id}`)}
                <span className="ml-1.5 text-[9px] font-mono text-content-tertiary">{preset.servers.length}</span>
              </button>
            );
          })}
        </div>
      </section>

      {/* Granted Servers */}
      <section>
        <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
          <Server className="text-brand" size={16} />
          <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">
            {t('plugin_workspace.granted_servers')}
          </h2>
        </div>
        {grantedServers.length === 0 ? (
          <div className="py-8 text-center text-content-tertiary bg-glass rounded-lg border border-edge border-dashed font-mono text-xs">
            {t('plugin_workspace.no_servers_granted')}
          </div>
        ) : (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-3">
            {grantedServers.map((server) => {
              const mgp = isMgpServer(server);
              const isTransitioning =
                server.status === 'Connecting' || server.status === 'Restarting' || server.status === 'Registered';
              const shimmer = isTransitioning ? (mgp ? 'shimmer-active-mgp' : 'shimmer-active') : '';
              return (
                <button
                  key={server.id}
                  type="button"
                  className={`text-left relative p-4 rounded-xl border transition-all duration-200 group cursor-pointer ${shimmer} ${
                    mgp
                      ? 'overflow-hidden border-purple-500/50 bg-purple-950/30 hover:border-red-500 shadow-purple-500/20 shadow-lg'
                      : 'border-edge card-solid hover:border-red-500'
                  }`}
                  aria-label={`${t('plugin_workspace.revoke')} ${displayServerId(server.id)}`}
                  onClick={() => onRevoke(server.id)}
                >
                  {mgp && (
                    <div
                      className="absolute inset-0 rounded-xl bg-gradient-to-br from-purple-500/20 via-transparent to-blue-500/10 pointer-events-none animate-pulse"
                      style={{ animationDuration: '3s' }}
                    />
                  )}
                  <span className="absolute top-2 right-2 p-1 rounded text-content-tertiary group-hover:text-red-500 opacity-0 group-hover:opacity-100 transition-all">
                    <X size={12} />
                  </span>
                  <div className={`flex items-center gap-2.5 mb-2 ${mgp ? 'relative' : ''}`}>
                    <Server
                      size={14}
                      className={`shrink-0 transition-colors ${mgp ? 'text-purple-400 group-hover:text-purple-300' : ''}`}
                      style={!mgp ? { color: agentColorHex } : undefined}
                    />
                    <span className="text-xs font-mono font-bold text-content-primary truncate">
                      {displayServerId(server.id)}
                    </span>
                    {mgp && (
                      <span
                        className="text-[9px] font-bold tracking-wider text-purple-400 shrink-0 drop-shadow-[0_0_4px_rgba(168,85,247,0.6)]"
                        title="MGP (bidirectional protocol)"
                      >
                        MGP
                      </span>
                    )}
                    {server.transport === 'streamable-http' && (
                      <span className="text-[9px] font-mono text-cyan-500/70 shrink-0" title="Remote HTTP transport">
                        HTTP
                      </span>
                    )}
                  </div>
                  <div
                    className={`flex items-center gap-3 text-[10px] font-mono text-content-tertiary leading-none ${mgp ? 'relative' : ''}`}
                  >
                    <span className="inline-flex items-center gap-1.5">
                      <StatusDot status={mcpStatusToDot(server)} size="sm" />
                      <span>{serverStatusLabel(server, t)}</span>
                    </span>
                    <span>{t('plugin_workspace.tools_count', { count: server.tools.length })}</span>
                  </div>
                </button>
              );
            })}
          </div>
        )}
      </section>

      {/* Available Servers */}
      {availableServers.length > 0 && (
        <section>
          <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
            <Plus className="text-brand" size={16} />
            <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">
              {t('plugin_workspace.available')}
            </h2>
          </div>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-3">
            {availableServers.map((server) => {
              const mgp = isMgpServer(server);
              const isTransitioning =
                server.status === 'Connecting' || server.status === 'Restarting' || server.status === 'Registered';
              const shimmer = isTransitioning ? (mgp ? 'shimmer-active-mgp' : 'shimmer-active') : '';
              return (
                <button
                  key={server.id}
                  type="button"
                  className={`text-left relative p-4 rounded-xl border transition-all duration-200 group cursor-pointer ${shimmer} ${
                    mgp
                      ? 'overflow-hidden border-purple-500/50 bg-purple-950/30 hover:bg-purple-950/40 hover:border-brand shadow-purple-500/20 shadow-lg'
                      : 'border-edge bg-surface-primary/30 hover:bg-surface-primary/50 hover:border-brand'
                  }`}
                  aria-label={`${t('plugin_workspace.grant')} ${displayServerId(server.id)}`}
                  onClick={() => onGrant(server.id)}
                >
                  {mgp && (
                    <div
                      className="absolute inset-0 rounded-xl bg-gradient-to-br from-purple-500/20 via-transparent to-blue-500/10 pointer-events-none animate-pulse"
                      style={{ animationDuration: '3s' }}
                    />
                  )}
                  <span className="absolute top-2 right-2 p-1 rounded text-content-tertiary group-hover:text-brand opacity-0 group-hover:opacity-100 transition-all">
                    <Plus size={12} />
                  </span>
                  <div className={`flex items-center gap-2.5 mb-2 ${mgp ? 'relative' : ''}`}>
                    <Server
                      size={14}
                      className={`shrink-0 transition-colors ${
                        mgp
                          ? 'text-purple-400 group-hover:text-purple-300'
                          : 'text-content-tertiary group-hover:text-brand'
                      }`}
                    />
                    <span className="text-xs font-mono font-bold text-content-primary truncate">
                      {displayServerId(server.id)}
                    </span>
                    {mgp && (
                      <span
                        className="text-[9px] font-bold tracking-wider text-purple-400 shrink-0 drop-shadow-[0_0_4px_rgba(168,85,247,0.6)]"
                        title="MGP (bidirectional protocol)"
                      >
                        MGP
                      </span>
                    )}
                    {server.transport === 'streamable-http' && (
                      <span className="text-[9px] font-mono text-cyan-500/70 shrink-0" title="Remote HTTP transport">
                        HTTP
                      </span>
                    )}
                  </div>
                  <div
                    className={`flex items-center gap-3 text-[10px] font-mono text-content-tertiary leading-none ${mgp ? 'relative' : ''}`}
                  >
                    <span className="inline-flex items-center gap-1.5">
                      <StatusDot status={mcpStatusToDot(server)} size="sm" />
                      <span>{serverStatusLabel(server, t)}</span>
                    </span>
                    <span>{t('plugin_workspace.tools_count', { count: server.tools.length })}</span>
                  </div>
                </button>
              );
            })}
          </div>
        </section>
      )}
    </>
  );
}
