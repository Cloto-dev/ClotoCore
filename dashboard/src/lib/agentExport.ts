import type { AccessControlEntry, AgentMetadata } from '../types';

interface ExportApi {
  getAgentAccess: (agentId: string) => Promise<{ entries: AccessControlEntry[] }>;
}

/**
 * Download an agent as a `.cloto-agent.json` file: what it is, and which
 * servers it is granted. Fields the kernel derives (avatar and password flags)
 * are left out, because an import that carried them would claim an avatar or a
 * password the new agent does not have.
 */
export async function exportAgent(api: ExportApi, agent: AgentMetadata): Promise<void> {
  try {
    const accessData = await api.getAgentAccess(agent.id);
    const mcpAccess = (accessData.entries || [])
      .filter((e) => e.entry_type === 'server_grant')
      .map((e) => ({ server_id: e.server_id, permission: e.permission }));

    const { has_avatar, avatar_description, has_power_password, has_password, ...cleanMeta } = agent.metadata || {};
    const exportData = {
      cloto_agent_export: 1,
      exported_at: new Date().toISOString(),
      agent: {
        name: agent.name,
        description: agent.description,
        default_engine_id: agent.default_engine_id || null,
        metadata: cleanMeta,
        required_capabilities: agent.required_capabilities,
      },
      mcp_access: mcpAccess,
      avatar_path: has_avatar === 'true' ? `avatars/${agent.id}.png` : null,
    };

    const blob = new Blob([JSON.stringify(exportData, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${agent.name}.cloto-agent.json`;
    a.click();
    URL.revokeObjectURL(url);
  } catch (e) {
    if (import.meta.env.DEV) console.error('Export failed:', e);
  }
}
