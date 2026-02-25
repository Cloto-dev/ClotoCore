import { useState } from 'react';
import { AccessControlEntry, AccessPermission } from '../../types';
import { ChevronDown, ChevronRight, Key, FolderOpen, Wrench } from 'lucide-react';

interface Props {
  entries: AccessControlEntry[];
  tools: string[];
  agentId: string;
  serverId: string;
  onChange: (entries: AccessControlEntry[]) => void;
}

function PermissionSelect({ value, inherited, onChange }: {
  value: AccessPermission | null;
  inherited: boolean;
  onChange: (v: AccessPermission | 'inherit') => void;
}) {
  return (
    <select
      value={inherited ? 'inherit' : (value ?? 'inherit')}
      onChange={e => onChange(e.target.value as AccessPermission | 'inherit')}
      className={`text-[10px] font-mono rounded px-1.5 py-0.5 border transition-colors
        ${value === 'allow' && !inherited ? 'border-green-500/30 bg-green-500/10 text-green-500' :
          value === 'deny' && !inherited ? 'border-red-500/30 bg-red-500/10 text-red-500' :
          'border-edge bg-glass text-content-muted'}`}
    >
      <option value="inherit">(inherited)</option>
      <option value="allow">Allow</option>
      <option value="deny">Deny</option>
    </select>
  );
}

export function McpAccessTree({ entries, tools, agentId, serverId, onChange }: Props) {
  const [expanded, setExpanded] = useState(true);

  // Find server_grant for this agent+server
  const serverGrant = entries.find(
    e => e.entry_type === 'server_grant' && e.agent_id === agentId && e.server_id === serverId && !e.tool_name
  );

  // Find tool_grants for this agent+server
  const toolGrants = new Map<string, AccessControlEntry>();
  entries
    .filter(e => e.entry_type === 'tool_grant' && e.agent_id === agentId && e.server_id === serverId && e.tool_name)
    .forEach(e => toolGrants.set(e.tool_name!, e));

  // Find capabilities for this agent
  const capabilities = entries.filter(
    e => e.entry_type === 'capability' && e.agent_id === agentId
  );

  function handleServerGrantChange(permission: AccessPermission | 'inherit') {
    const now = new Date().toISOString();
    let updated = entries.filter(
      e => !(e.entry_type === 'server_grant' && e.agent_id === agentId && e.server_id === serverId && !e.tool_name)
    );
    if (permission !== 'inherit') {
      updated.push({
        entry_type: 'server_grant',
        agent_id: agentId,
        server_id: serverId,
        permission,
        granted_by: 'user',
        granted_at: now,
      });
    }
    onChange(updated);
  }

  function handleToolGrantChange(toolName: string, permission: AccessPermission | 'inherit') {
    const now = new Date().toISOString();
    let updated = entries.filter(
      e => !(e.entry_type === 'tool_grant' && e.agent_id === agentId && e.server_id === serverId && e.tool_name === toolName)
    );
    if (permission !== 'inherit') {
      updated.push({
        entry_type: 'tool_grant',
        agent_id: agentId,
        server_id: serverId,
        tool_name: toolName,
        permission,
        granted_by: 'user',
        granted_at: now,
      });
    }
    onChange(updated);
  }

  return (
    <div className="text-xs font-mono">
      {/* Capabilities */}
      {capabilities.map((cap, i) => (
        <div key={`cap-${i}`} className="flex items-center gap-2 py-1 px-1">
          <Key size={12} className="text-yellow-500 flex-shrink-0" />
          <span className="text-content-secondary">Capability: {cap.justification ?? cap.server_id}</span>
          <span className={`ml-auto text-[10px] px-1.5 py-0.5 rounded ${
            cap.permission === 'allow' ? 'bg-green-500/10 text-green-500' : 'bg-red-500/10 text-red-500'
          }`}>
            {cap.permission === 'allow' ? 'Approved' : 'Denied'}
          </span>
        </div>
      ))}

      {/* Server Grant */}
      <div>
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex items-center gap-1 py-1 px-1 w-full hover:bg-glass rounded transition-colors"
        >
          {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
          <FolderOpen size={12} className="text-blue-500 flex-shrink-0" />
          <span className="text-content-secondary">Server Grant: {serverId}</span>
          <div className="ml-auto" onClick={e => e.stopPropagation()}>
            <PermissionSelect
              value={serverGrant?.permission ?? null}
              inherited={!serverGrant}
              onChange={handleServerGrantChange}
            />
          </div>
        </button>

        {/* Tool Grants */}
        {expanded && tools.map(tool => {
          const grant = toolGrants.get(tool);
          return (
            <div key={tool} className="flex items-center gap-2 py-1 pl-7 pr-1">
              <Wrench size={12} className="text-content-tertiary flex-shrink-0" />
              <span className="text-content-secondary">{tool}</span>
              <div className="ml-auto">
                <PermissionSelect
                  value={grant?.permission ?? null}
                  inherited={!grant}
                  onChange={v => handleToolGrantChange(tool, v)}
                />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
