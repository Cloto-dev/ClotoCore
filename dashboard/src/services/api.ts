import { AgentMetadata, PluginManifest, ContentBlock, ChatMessage, ClotoMessage, PermissionRequest, Metrics, Memory, Episode, StrictSystemEvent, McpServerInfo, McpServerSettings, AccessTreeResponse, AccessControlEntry } from '../types';
import { isTauri } from '../lib/tauri';

// In Tauri mode, window.location.origin returns "tauri://localhost" which cannot reach
// the HTTP kernel. We must use the actual loopback address with the kernel port.
const KERNEL_PORT = 8081;
const API_URL = import.meta.env.VITE_API_URL
  || (isTauri ? `http://127.0.0.1:${KERNEL_PORT}/api` : `${window.location.origin}/api`);
export const API_BASE = API_URL.endsWith('/api') ? API_URL : `${API_URL}/api`;
export const EVENTS_URL = `${API_BASE}/events`;

/** Throw with detailed error message from JSON body if available */
async function throwIfNotOk(res: Response, ctx: string): Promise<void> {
  if (res.ok) return;
  const body = await res.json().catch(() => ({}));
  throw new Error(body?.error?.message || `Failed to ${ctx}: ${res.statusText}`);
}

async function fetchJson<T>(path: string, ctx: string): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`);
  if (!res.ok) throw new Error(`Failed to ${ctx}: ${res.statusText}`);
  return res.json();
}

async function mutate(
  path: string, method: string, ctx: string,
  body?: unknown, extraHeaders?: Record<string, string>,
): Promise<Response> {
  const res = await fetch(`${API_BASE}${path}`, {
    method,
    headers: { 'Content-Type': 'application/json', ...extraHeaders },
    ...(body !== undefined && { body: JSON.stringify(body) }),
  });
  if (!res.ok) throw new Error(`Failed to ${ctx}: ${res.statusText}`);
  return res;
}

export const api = {
  getAgents: () => fetchJson<AgentMetadata[]>('/agents', 'fetch agents'),
  getPlugins: () => fetchJson<PluginManifest[]>('/plugins', 'fetch plugins'),
  getPluginConfig: (id: string, apiKey: string) => {
    const res = fetch(`${API_BASE}/plugins/${id}/config`, {
      headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
    });
    return res.then(r => { if (!r.ok) throw new Error(`Failed to get plugin config: ${r.statusText}`); return r.json() as Promise<Record<string, string>>; });
  },
  getPendingPermissions: () => fetchJson<PermissionRequest[]>('/permissions/pending', 'fetch pending permissions'),
  getVersion: () => fetchJson<{ version: string; build_target: string }>('/system/version', 'fetch version'),
  getMetrics: () => fetchJson<Metrics>('/metrics', 'fetch metrics'),
  getMemories: () => fetchJson<Memory[]>('/memories', 'fetch memories'),
  getEpisodes: () => fetchJson<Episode[]>('/episodes', 'fetch episodes'),
  getHistory: () => fetchJson<StrictSystemEvent[]>('/history', 'fetch history'),
  fetchJson: <T>(path: string, apiKey: string) =>
    fetch(`${API_BASE}${path}`, { headers: { 'X-API-Key': apiKey } })
      .then(r => { if (!r.ok) throw new Error(`${r.statusText}`); return r.json() as Promise<T>; }),
  put: (path: string, body: unknown, apiKey: string) =>
    mutate(path, 'PUT', path, body, { 'X-API-Key': apiKey }).then(r => r.json()),
  applyPluginSettings: (settings: { id: string, is_active: boolean }[], apiKey: string) =>
    mutate('/plugins/apply', 'POST', 'apply plugin settings', settings, { 'X-API-Key': apiKey }).then(() => {}),
  updatePluginConfig: (id: string, payload: { key: string, value: string }, apiKey: string) =>
    mutate(`/plugins/${id}/config`, 'POST', 'update plugin config', payload, { 'X-API-Key': apiKey }).then(() => {}),
  updateAgent: (id: string, payload: { default_engine_id?: string, metadata: Record<string, string> }, apiKey: string) =>
    mutate(`/agents/${id}`, 'POST', 'update agent', payload, { 'X-API-Key': apiKey }).then(() => {}),

  getAgentPlugins: async (agentId: string, apiKey: string): Promise<{ plugin_id: string; pos_x: number; pos_y: number }[]> => {
    const res = await fetch(`${API_BASE}/agents/${agentId}/plugins`, {
      headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
    });
    if (!res.ok) throw new Error(`Failed to get agent plugins: ${res.statusText}`);
    const data = await res.json();
    return data.plugins ?? [];
  },

  setAgentPlugins: async (agentId: string, plugins: { plugin_id: string; pos_x: number; pos_y: number }[], apiKey: string): Promise<void> => {
    const res = await fetch(`${API_BASE}/agents/${agentId}/plugins`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
      body: JSON.stringify({ plugins }),
    });
    await throwIfNotOk(res, 'set agent plugins');
  },
  getPluginPermissions: async (pluginId: string, apiKey: string): Promise<string[]> => {
    const res = await fetch(`${API_BASE}/plugins/${pluginId}/permissions`, {
      headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
    });
    if (!res.ok) throw new Error(`Failed to get permissions: ${res.statusText}`);
    const data = await res.json();
    return data.permissions ?? [];
  },

  revokePermission: async (pluginId: string, permission: string, apiKey: string): Promise<void> => {
    const res = await fetch(`${API_BASE}/plugins/${pluginId}/permissions`, {
      method: 'DELETE',
      headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
      body: JSON.stringify({ permission }),
    });
    await throwIfNotOk(res, 'revoke permission');
  },

  grantPermission: (pluginId: string, permission: string, apiKey: string) =>
    mutate(`/plugins/${pluginId}/permissions/grant`, 'POST', 'grant permission', { permission }, { 'X-API-Key': apiKey }).then(() => {}),
  postEvent: (eventData: unknown, apiKey: string) =>
    mutate('/events/publish', 'POST', 'post event', eventData, { 'X-API-Key': apiKey }).then(() => {}),
  post: (path: string, payload: unknown, apiKey: string) =>
    mutate(path, 'POST', `post to ${path}`, payload, { 'X-API-Key': apiKey }).then(() => {}),
  approvePermission: (requestId: string, approvedBy: string, apiKey: string) =>
    mutate(`/permissions/${requestId}/approve`, 'POST', 'approve permission', { approved_by: approvedBy }, { 'X-API-Key': apiKey }).then(() => {}),
  denyPermission: (requestId: string, approvedBy: string, apiKey: string) =>
    mutate(`/permissions/${requestId}/deny`, 'POST', 'deny permission', { approved_by: approvedBy }, { 'X-API-Key': apiKey }).then(() => {}),
  async deleteAgent(agentId: string, apiKey: string): Promise<void> {
    const res = await fetch(`${API_BASE}/agents/${agentId}`, {
      method: 'DELETE',
      headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
    });
    await throwIfNotOk(res, 'delete agent');
  },

  async createAgent(payload: { name: string; description: string; default_engine: string; metadata: Record<string, string>; password?: string }, apiKey: string): Promise<void> {
    const res = await fetch(`${API_BASE}/agents`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
      body: JSON.stringify(payload),
    });
    await throwIfNotOk(res, 'create agent');
  },
  postChat: (message: ClotoMessage, apiKey: string) =>
    mutate('/chat', 'POST', 'send chat', message, { 'X-API-Key': apiKey }).then(() => {}),
  postChatMessage: (agentId: string, msg: { id: string; source: string; content: ContentBlock[]; metadata?: Record<string, unknown> }, apiKey: string): Promise<{ id: string; created_at: number }> =>
    mutate(`/chat/${agentId}/messages`, 'POST', 'post chat message', msg, { 'X-API-Key': apiKey }).then(r => r.json()),
  deleteChatMessages: (agentId: string, apiKey: string): Promise<{ deleted_count: number }> =>
    mutate(`/chat/${agentId}/messages`, 'DELETE', 'delete chat messages', undefined, { 'X-API-Key': apiKey }).then(r => r.json()),
  invalidateApiKey: (apiKey: string): Promise<{ status: string; message: string }> =>
    mutate('/system/invalidate-key', 'POST', 'invalidate API key', undefined, { 'X-API-Key': apiKey }).then(r => r.json()),

  // Custom error handling: reads error body for detailed message
  async toggleAgentPower(agentId: string, enabled: boolean, apiKey: string, password?: string): Promise<void> {
    const res = await fetch(`${API_BASE}/agents/${agentId}/power`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
      body: JSON.stringify({ enabled, password: password || undefined })
    });
    await throwIfNotOk(res, 'toggle agent power');
  },

  // Custom response transformation: parses JSON string fields
  async getChatMessages(agentId: string, apiKey: string, before?: number, limit?: number): Promise<{ messages: ChatMessage[], has_more: boolean }> {
    const params = new URLSearchParams();
    if (before) params.set('before', String(before));
    if (limit) params.set('limit', String(limit));
    const qs = params.toString();
    const res = await fetch(`${API_BASE}/chat/${agentId}/messages${qs ? '?' + qs : ''}`, {
      headers: { 'X-API-Key': apiKey },
    });
    if (!res.ok) throw new Error(`Failed to fetch chat messages: ${res.statusText}`);
    const data = await res.json();
    return {
      messages: data.messages.map((m: any) => ({
        ...m,
        content: typeof m.content === 'string' ? JSON.parse(m.content) : m.content,
        metadata: m.metadata ? (typeof m.metadata === 'string' ? JSON.parse(m.metadata) : m.metadata) : undefined,
      })),
      has_more: data.has_more,
    };
  },

  getAttachmentUrl(attachmentId: string): string {
    return `${API_BASE}/chat/attachments/${attachmentId}`;
  },

  // MCP Server Management (MCP_SERVER_UI_DESIGN.md §4)
  listMcpServers: async (apiKey: string): Promise<{ servers: McpServerInfo[]; count: number }> => {
    const res = await fetch(`${API_BASE}/mcp/servers`, {
      headers: { 'X-API-Key': apiKey },
    });
    if (!res.ok) throw new Error(`Failed to list MCP servers: ${res.statusText}`);
    return res.json();
  },

  getMcpServerSettings: async (name: string, apiKey: string): Promise<McpServerSettings> => {
    const res = await fetch(`${API_BASE}/mcp/servers/${encodeURIComponent(name)}/settings`, {
      headers: { 'X-API-Key': apiKey },
    });
    if (!res.ok) throw new Error(`Failed to get server settings: ${res.statusText}`);
    return res.json();
  },

  updateMcpServerSettings: (name: string, settings: { default_policy?: string }, apiKey: string) =>
    mutate(`/mcp/servers/${encodeURIComponent(name)}/settings`, 'PUT', 'update server settings', settings, { 'X-API-Key': apiKey }).then(() => {}),

  getMcpServerAccess: async (name: string, apiKey: string): Promise<AccessTreeResponse> => {
    const res = await fetch(`${API_BASE}/mcp/servers/${encodeURIComponent(name)}/access`, {
      headers: { 'X-API-Key': apiKey },
    });
    if (!res.ok) throw new Error(`Failed to get access control: ${res.statusText}`);
    return res.json();
  },

  putMcpServerAccess: (name: string, entries: AccessControlEntry[], apiKey: string) =>
    mutate(`/mcp/servers/${encodeURIComponent(name)}/access`, 'PUT', 'update access control', { entries }, { 'X-API-Key': apiKey }).then(() => {}),

  getAgentAccess: (agentId: string) =>
    fetchJson<{ agent_id: string; entries: AccessControlEntry[] }>(`/mcp/access/by-agent/${encodeURIComponent(agentId)}`, 'fetch agent access'),

  startMcpServer: (name: string, apiKey: string) =>
    mutate(`/mcp/servers/${encodeURIComponent(name)}/start`, 'POST', 'start MCP server', undefined, { 'X-API-Key': apiKey }).then(r => r.json()),

  stopMcpServer: (name: string, apiKey: string) =>
    mutate(`/mcp/servers/${encodeURIComponent(name)}/stop`, 'POST', 'stop MCP server', undefined, { 'X-API-Key': apiKey }).then(r => r.json()),

  restartMcpServer: (name: string, apiKey: string) =>
    mutate(`/mcp/servers/${encodeURIComponent(name)}/restart`, 'POST', 'restart MCP server', undefined, { 'X-API-Key': apiKey }).then(r => r.json()),

  createMcpServer: (payload: { name: string; command?: string; args?: string[]; code?: string; description?: string }, apiKey: string) =>
    mutate('/mcp/servers', 'POST', 'create MCP server', payload, { 'X-API-Key': apiKey }).then(r => r.json()),

  deleteMcpServer: (name: string, apiKey: string) =>
    mutate(`/mcp/servers/${encodeURIComponent(name)}`, 'DELETE', 'delete MCP server', undefined, { 'X-API-Key': apiKey }).then(() => {}),
};
