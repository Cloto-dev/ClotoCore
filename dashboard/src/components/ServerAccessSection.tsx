import { Server, Plus, X, Wifi, WifiOff, AlertTriangle } from 'lucide-react';
import { McpServerInfo } from '../types';
import { agentColor } from '../lib/agentIdentity';

const StatusIcon = ({ status }: { status: McpServerInfo['status'] }) => {
  switch (status) {
    case 'Connected': return <Wifi size={12} className="text-emerald-500" />;
    case 'Disconnected': return <WifiOff size={12} className="text-content-muted" />;
    case 'Error': return <AlertTriangle size={12} className="text-red-500" />;
  }
};

const StatusBadge = ({ status }: { status: McpServerInfo['status'] }) => (
  <span className={`text-[9px] font-bold uppercase tracking-wider px-2 py-0.5 rounded ${
    status === 'Connected' ? 'bg-emerald-500/10 text-emerald-500' :
    status === 'Error' ? 'bg-red-500/10 text-red-500' :
    'bg-surface-secondary text-content-tertiary'
  }`}>
    {status}
  </span>
);

interface Props {
  grantedServers: McpServerInfo[];
  availableServers: McpServerInfo[];
  agentColorHex: string;
  onGrant: (serverId: string) => void;
  onRevoke: (serverId: string) => void;
}

export function ServerAccessSection({ grantedServers, availableServers, agentColorHex, onGrant, onRevoke }: Props) {
  return (
    <>
      {/* Granted Servers */}
      <section>
        <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
          <Server className="text-brand" size={16} />
          <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">Granted Servers</h2>
        </div>
        {grantedServers.length === 0 ? (
          <div className="py-8 text-center text-content-tertiary bg-glass rounded-lg border border-edge border-dashed font-mono text-xs">
            No servers granted. Add from the list below.
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
                  <span className="text-xs font-bold text-content-primary">{server.id}</span>
                  <span className="text-[10px] text-content-tertiary ml-2 font-mono">{server.tools.length} tools</span>
                </div>
                <StatusIcon status={server.status} />
                <StatusBadge status={server.status} />
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
            <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">Available</h2>
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
                  <span className="text-xs font-medium text-content-secondary">{server.id}</span>
                  <span className="text-[10px] text-content-tertiary ml-2 font-mono">{server.tools.length} tools</span>
                </div>
                <StatusIcon status={server.status} />
                <StatusBadge status={server.status} />
                <span className="inline-flex items-center gap-1 px-2 py-1 rounded text-[10px] font-bold text-brand opacity-0 group-hover:opacity-100 transition-all">
                  <Plus size={10} /> Grant
                </span>
              </div>
            ))}
          </div>
        </section>
      )}
    </>
  );
}
