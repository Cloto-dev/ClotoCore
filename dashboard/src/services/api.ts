import { safeJsonParse } from '../lib/json';
import { isTauri } from '../lib/tauri';
import type {
  AccessControlEntry,
  AccessTreeResponse,
  AgentMetadata,
  ChatMessage,
  ClotoMessage,
  ContentBlock,
  Episode,
  MarketplaceCatalogEntry,
  McpServerInfo,
  McpServerSettings,
  Memory,
  MemoryCapabilities,
  Metrics,
  PermissionRequest,
  SetupStatus,
  StrictSystemEvent,
} from '../types';

// In Tauri mode, window.location.origin returns "tauri://localhost" which cannot reach
// the HTTP kernel. We must use the actual loopback address with the kernel port.
const KERNEL_PORT = 8081;
const API_URL =
  import.meta.env.VITE_API_URL || (isTauri ? `http://127.0.0.1:${KERNEL_PORT}/api` : `${window.location.origin}/api`);
export const API_BASE = API_URL.endsWith('/api') ? API_URL : `${API_URL}/api`;
export const EVENTS_URL = `${API_BASE}/events`;

const HEALTH_CHECK_TIMEOUT_MS = 3000;
const API_TIMEOUT_MS = 15_000;

// Health check types
export interface HealthCheck {
  name: string;
  status: 'healthy' | 'degraded' | 'error';
  message: string;
  repairable: boolean;
  detail?: Record<string, unknown>;
}

export interface HealthReport {
  status: 'healthy' | 'degraded' | 'error';
  checks: HealthCheck[];
  timestamp: string;
  db_size_bytes: number;
}

export interface RepairAction {
  name: string;
  fixed_count: number;
  message: string;
}

export interface RepairReport {
  actions: RepairAction[];
  total_fixed: number;
}

/** Throw with detailed error message from JSON body if available */
async function throwIfNotOk(res: Response, ctx: string): Promise<void> {
  if (res.ok) return;
  const body = await res.json().catch(() => ({}));
  throw new Error(body?.error?.message || `Failed to ${ctx}: ${res.statusText}`);
}

async function fetchJson<T>(path: string, ctx: string, apiKey?: string, signal?: AbortSignal): Promise<T> {
  const headers: Record<string, string> = {};
  if (apiKey) headers['X-API-Key'] = apiKey;
  const res = await fetch(`${API_BASE}${path}`, { headers, signal: signal ?? AbortSignal.timeout(API_TIMEOUT_MS) });
  if (!res.ok) throw new Error(`Failed to ${ctx}: ${res.statusText}`);
  const body = await res.json();
  return body.data as T;
}

async function mutate(
  path: string,
  method: string,
  ctx: string,
  body?: unknown,
  extraHeaders?: Record<string, string>,
  signal?: AbortSignal,
): Promise<Response> {
  const res = await fetch(`${API_BASE}${path}`, {
    method,
    headers: { 'Content-Type': 'application/json', ...extraHeaders },
    signal: signal ?? AbortSignal.timeout(API_TIMEOUT_MS),
    ...(body !== undefined && { body: JSON.stringify(body) }),
  });
  if (!res.ok) throw new Error(`Failed to ${ctx}: ${res.statusText}`);
  return res;
}

export const api = {
  getHealth: async (): Promise<{ status: string }> => {
    const res = await fetch(`${API_BASE}/system/health`, { signal: AbortSignal.timeout(HEALTH_CHECK_TIMEOUT_MS) });
    if (!res.ok) throw new Error(res.statusText);
    return res.json().then((b) => b.data);
  },

  getAgents: (apiKey?: string) => fetchJson<AgentMetadata[]>('/agents', 'fetch agents', apiKey),
  getPendingPermissions: (apiKey?: string) =>
    fetchJson<PermissionRequest[]>('/permissions/pending', 'fetch pending permissions', apiKey),
  getVersion: () => fetchJson<{ version: string; build_target: string }>('/system/version', 'fetch version'),
  getMetrics: (apiKey?: string) => fetchJson<Metrics>('/metrics', 'fetch metrics', apiKey),
  getMemories: async (apiKey?: string): Promise<{ memories: Memory[]; capabilities: MemoryCapabilities }> => {
    const data = await fetchJson<{ memories: Memory[]; count: number; capabilities?: MemoryCapabilities }>(
      '/memories',
      'fetch memories',
      apiKey,
    );
    return {
      memories: data.memories ?? [],
      capabilities: data.capabilities ?? { update_memory: false, lock_memory: false, unlock_memory: false },
    };
  },
  getEpisodes: async (apiKey?: string): Promise<Episode[]> => {
    const data = await fetchJson<{ episodes: Episode[]; count: number }>('/episodes', 'fetch episodes', apiKey);
    return data.episodes ?? [];
  },
  getHistory: (apiKey?: string) => fetchJson<StrictSystemEvent[]>('/history', 'fetch history', apiKey),
  getPlugins: (apiKey?: string) => fetchJson<unknown[]>('/plugins', 'fetch plugins', apiKey),
  fetchJson: <T>(path: string, apiKey: string) =>
    fetch(`${API_BASE}${path}`, { headers: { 'X-API-Key': apiKey } }).then((r) => {
      if (!r.ok) throw new Error(`${r.statusText}`);
      return r.json().then((b: { data: T }) => b.data);
    }),
  put: (path: string, body: unknown, apiKey: string) =>
    mutate(path, 'PUT', path, body, { 'X-API-Key': apiKey })
      .then((r) => r.json())
      .then((b) => b.data),
  updateAgent: (
    id: string,
    payload: { name?: string; description?: string; default_engine_id?: string; metadata?: Record<string, string> },
    apiKey: string,
  ) => mutate(`/agents/${id}`, 'POST', 'update agent', payload, { 'X-API-Key': apiKey }).then(() => {}),

  getPluginPermissions: async (pluginId: string, apiKey: string): Promise<string[]> => {
    const res = await fetch(`${API_BASE}/plugins/${pluginId}/permissions`, {
      headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
    });
    if (!res.ok) throw new Error(`Failed to get permissions: ${res.statusText}`);
    const data = (await res.json()).data;
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
    mutate(
      `/plugins/${pluginId}/permissions/grant`,
      'POST',
      'grant permission',
      { permission },
      { 'X-API-Key': apiKey },
    ).then(() => {}),
  postEvent: (eventData: unknown, apiKey: string) =>
    mutate('/events/publish', 'POST', 'post event', eventData, { 'X-API-Key': apiKey }).then(() => {}),
  post: (path: string, payload: unknown, apiKey: string) =>
    mutate(path, 'POST', `post to ${path}`, payload, { 'X-API-Key': apiKey }).then(() => {}),
  approvePermission: (requestId: string, approvedBy: string, apiKey: string) =>
    mutate(
      `/permissions/${requestId}/approve`,
      'POST',
      'approve permission',
      { approved_by: approvedBy },
      { 'X-API-Key': apiKey },
    ).then(() => {}),
  denyPermission: (requestId: string, approvedBy: string, apiKey: string) =>
    mutate(
      `/permissions/${requestId}/deny`,
      'POST',
      'deny permission',
      { approved_by: approvedBy },
      { 'X-API-Key': apiKey },
    ).then(() => {}),
  // Command approval
  approveCommand: (approvalId: string, apiKey: string) =>
    mutate(
      `/commands/${encodeURIComponent(approvalId)}/approve`,
      'POST',
      'approve command',
      {},
      { 'X-API-Key': apiKey },
    ).then(() => {}),
  trustCommand: (approvalId: string, apiKey: string) =>
    mutate(
      `/commands/${encodeURIComponent(approvalId)}/trust`,
      'POST',
      'trust command',
      {},
      { 'X-API-Key': apiKey },
    ).then(() => {}),
  denyCommand: (approvalId: string, apiKey: string) =>
    mutate(
      `/commands/${encodeURIComponent(approvalId)}/deny`,
      'POST',
      'deny command',
      {},
      { 'X-API-Key': apiKey },
    ).then(() => {}),
  async deleteAgent(agentId: string, apiKey: string, password?: string): Promise<void> {
    const res = await fetch(`${API_BASE}/agents/${agentId}`, {
      method: 'DELETE',
      headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
      ...(password ? { body: JSON.stringify({ password }) } : {}),
    });
    await throwIfNotOk(res, 'delete agent');
  },

  async createAgent(
    payload: {
      name: string;
      description: string;
      default_engine: string;
      metadata: Record<string, string>;
      password?: string;
    },
    apiKey: string,
  ): Promise<void> {
    const res = await fetch(`${API_BASE}/agents`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
      body: JSON.stringify(payload),
    });
    await throwIfNotOk(res, 'create agent');
  },
  postChat: (message: ClotoMessage, apiKey: string) =>
    mutate('/chat', 'POST', 'send chat', message, { 'X-API-Key': apiKey }).then(() => {}),
  postChatMessage: (
    agentId: string,
    msg: { id: string; source: string; content: ContentBlock[]; metadata?: Record<string, unknown> },
    apiKey: string,
  ): Promise<{ id: string; created_at: number }> =>
    mutate(`/chat/${agentId}/messages`, 'POST', 'post chat message', msg, { 'X-API-Key': apiKey })
      .then((r) => r.json())
      .then((b) => b.data),
  deleteChatMessages: (agentId: string, apiKey: string, userId?: string): Promise<{ deleted_count: number }> => {
    const qs = userId ? `?user_id=${encodeURIComponent(userId)}` : '';
    return mutate(`/chat/${agentId}/messages${qs}`, 'DELETE', 'delete chat messages', undefined, {
      'X-API-Key': apiKey,
    })
      .then((r) => r.json())
      .then((b) => b.data);
  },
  retryResponse: (agentId: string, messageId: string, apiKey: string): Promise<{ retry_id: string }> =>
    mutate(
      `/chat/${agentId}/messages/${encodeURIComponent(messageId)}/retry`,
      'POST',
      'retry response',
      {},
      { 'X-API-Key': apiKey },
    )
      .then((r) => r.json())
      .then((b) => b.data),
  invalidateApiKey: (apiKey: string): Promise<{ message: string }> =>
    mutate('/system/invalidate-key', 'POST', 'invalidate API key', undefined, { 'X-API-Key': apiKey })
      .then((r) => r.json())
      .then((b) => b.data),

  // Custom error handling: reads error body for detailed message
  async toggleAgentPower(agentId: string, enabled: boolean, apiKey: string, password?: string): Promise<void> {
    const res = await fetch(`${API_BASE}/agents/${agentId}/power`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
      body: JSON.stringify({ enabled, password: password || undefined }),
    });
    await throwIfNotOk(res, 'toggle agent power');
  },

  // Custom response transformation: parses JSON string fields
  async getChatMessages(
    agentId: string,
    apiKey: string,
    before?: number,
    limit?: number,
    userId?: string,
  ): Promise<{ messages: ChatMessage[]; has_more: boolean }> {
    const params = new URLSearchParams();
    if (before) params.set('before', String(before));
    if (limit) params.set('limit', String(limit));
    if (userId) params.set('user_id', userId);
    const qs = params.toString();
    const res = await fetch(`${API_BASE}/chat/${agentId}/messages${qs ? '?' + qs : ''}`, {
      headers: { 'X-API-Key': apiKey },
    });
    if (!res.ok) throw new Error(`Failed to fetch chat messages: ${res.statusText}`);
    const data = (await res.json()).data;
    return {
      messages: (data.messages as Array<Record<string, unknown>>).map((m) => ({
        ...m,
        content: typeof m.content === 'string' ? safeJsonParse(m.content, m.content) : m.content,
        metadata: m.metadata
          ? typeof m.metadata === 'string'
            ? safeJsonParse(m.metadata, {})
            : m.metadata
          : undefined,
      })) as ChatMessage[],
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
    return res.json().then((b) => b.data);
  },

  getMcpServerSettings: async (name: string, apiKey: string): Promise<McpServerSettings> => {
    const res = await fetch(`${API_BASE}/mcp/servers/${encodeURIComponent(name)}/settings`, {
      headers: { 'X-API-Key': apiKey },
    });
    if (!res.ok) throw new Error(`Failed to get server settings: ${res.statusText}`);
    return res.json().then((b) => b.data);
  },

  updateMcpServerSettings: (
    name: string,
    settings: { default_policy?: string; env?: Record<string, string> },
    apiKey: string,
  ) =>
    mutate(`/mcp/servers/${encodeURIComponent(name)}/settings`, 'PUT', 'update server settings', settings, {
      'X-API-Key': apiKey,
    }).then(() => {}),

  getMcpServerAccess: async (name: string, apiKey: string): Promise<AccessTreeResponse> => {
    const res = await fetch(`${API_BASE}/mcp/servers/${encodeURIComponent(name)}/access`, {
      headers: { 'X-API-Key': apiKey },
    });
    if (!res.ok) throw new Error(`Failed to get access control: ${res.statusText}`);
    return res.json().then((b) => b.data);
  },

  putMcpServerAccess: (name: string, entries: AccessControlEntry[], apiKey: string) =>
    mutate(
      `/mcp/servers/${encodeURIComponent(name)}/access`,
      'PUT',
      'update access control',
      { entries },
      { 'X-API-Key': apiKey },
    ).then(() => {}),

  /**
   * Replace all `server_grant` entries for an agent in a single request.
   * Used by bulk flows (AgentPluginWorkspace save, SetupWizard preset apply,
   * AgentTerminal import) to avoid the 2N REST-call pattern that tripped the
   * rate limiter on larger grant sets. Preserves tool_grant and capability
   * entries on the backend.
   */
  putAgentMcpAccess: (agentId: string, grantedServerIds: string[], apiKey: string) =>
    mutate(
      `/agents/${encodeURIComponent(agentId)}/mcp-access`,
      'PUT',
      'update agent MCP access',
      { granted_server_ids: grantedServerIds },
      { 'X-API-Key': apiKey },
    ).then(() => {}),

  getAgentAccess: (agentId: string, apiKey?: string) =>
    fetchJson<{ agent_id: string; entries: AccessControlEntry[] }>(
      `/mcp/access/by-agent/${encodeURIComponent(agentId)}`,
      'fetch agent access',
      apiKey,
    ),

  startMcpServer: (name: string, apiKey: string) =>
    mutate(`/mcp/servers/${encodeURIComponent(name)}/start`, 'POST', 'start MCP server', undefined, {
      'X-API-Key': apiKey,
    })
      .then((r) => r.json())
      .then((b) => b.data),

  stopMcpServer: (name: string, apiKey: string) =>
    mutate(`/mcp/servers/${encodeURIComponent(name)}/stop`, 'POST', 'stop MCP server', undefined, {
      'X-API-Key': apiKey,
    })
      .then((r) => r.json())
      .then((b) => b.data),

  restartMcpServer: (name: string, apiKey: string) =>
    mutate(`/mcp/servers/${encodeURIComponent(name)}/restart`, 'POST', 'restart MCP server', undefined, {
      'X-API-Key': apiKey,
    })
      .then((r) => r.json())
      .then((b) => b.data),

  createMcpServer: (
    payload: { name: string; command?: string; args?: string[]; code?: string; description?: string },
    apiKey: string,
  ) =>
    mutate('/mcp/servers', 'POST', 'create MCP server', payload, { 'X-API-Key': apiKey })
      .then((r) => r.json())
      .then((b) => b.data),

  deleteMcpServer: (name: string, apiKey: string) =>
    mutate(`/mcp/servers/${encodeURIComponent(name)}`, 'DELETE', 'delete MCP server', undefined, {
      'X-API-Key': apiKey,
    }).then(() => {}),

  // Cron Job Management (Layer 2: Autonomous Trigger)
  listCronJobs: (apiKey: string, agentId?: string): Promise<{ jobs: import('../types').CronJob[]; count: number }> => {
    const qs = agentId ? `?agent_id=${encodeURIComponent(agentId)}` : '';
    return fetch(`${API_BASE}/cron/jobs${qs}`, { headers: { 'X-API-Key': apiKey } }).then((r) => {
      if (!r.ok) throw new Error(r.statusText);
      return r.json().then((b) => b.data);
    });
  },

  createCronJob: (
    payload: {
      agent_id: string;
      name: string;
      schedule_type: string;
      schedule_value: string;
      message: string;
      engine_id?: string;
      max_iterations?: number;
      hide_prompt?: boolean;
      source_type?: 'user' | 'system';
      creator_user_id?: string;
      creator_user_name?: string;
    },
    apiKey: string,
  ) =>
    mutate('/cron/jobs', 'POST', 'create cron job', payload, { 'X-API-Key': apiKey })
      .then((r) => r.json())
      .then((b) => b.data),

  deleteCronJob: (jobId: string, apiKey: string) =>
    mutate(`/cron/jobs/${encodeURIComponent(jobId)}`, 'DELETE', 'delete cron job', undefined, {
      'X-API-Key': apiKey,
    }).then(() => {}),

  toggleCronJob: (jobId: string, enabled: boolean, apiKey: string) =>
    mutate(
      `/cron/jobs/${encodeURIComponent(jobId)}/toggle`,
      'POST',
      'toggle cron job',
      { enabled },
      { 'X-API-Key': apiKey },
    ).then(() => {}),

  runCronJobNow: (jobId: string, apiKey: string) =>
    mutate(`/cron/jobs/${encodeURIComponent(jobId)}/run`, 'POST', 'run cron job', undefined, { 'X-API-Key': apiKey })
      .then((r) => r.json())
      .then((b) => b.data),

  // LLM Provider Management (MGP §13.4)
  listLlmProviders: (
    apiKey: string,
  ): Promise<{
    providers: Array<{
      id: string;
      display_name: string;
      api_url: string;
      has_key: boolean;
      model_id: string;
      timeout_secs: number;
      enabled: boolean;
      context_length: number | null;
      thinking_mode: 'auto' | 'on' | 'off';
    }>;
  }> =>
    fetch(`${API_BASE}/llm/providers`, { headers: { 'X-API-Key': apiKey } }).then((r) => {
      if (!r.ok) throw new Error(r.statusText);
      return r.json().then((b) => b.data);
    }),

  setLlmProviderKey: (providerId: string, apiKey: string, providerApiKey: string) =>
    mutate(
      `/llm/providers/${encodeURIComponent(providerId)}/key`,
      'POST',
      'set provider key',
      { api_key: providerApiKey },
      { 'X-API-Key': apiKey },
    ).then(() => {}),

  deleteLlmProviderKey: (providerId: string, apiKey: string) =>
    mutate(`/llm/providers/${encodeURIComponent(providerId)}/key`, 'DELETE', 'delete provider key', undefined, {
      'X-API-Key': apiKey,
    }).then(() => {}),

  setLlmProviderModel: (providerId: string, apiKey: string, modelId: string) =>
    mutate(
      `/llm/providers/${encodeURIComponent(providerId)}/model`,
      'POST',
      'set provider model',
      { model_id: modelId },
      { 'X-API-Key': apiKey },
    ).then(() => {}),

  getAgentLastUsage: (
    agentId: string,
    apiKey: string,
  ): Promise<{
    usage: {
      prompt_tokens: number;
      completion_tokens: number;
      total_tokens: number;
      context_length: number | null;
      provider_id: string;
      model_id: string;
      is_estimate: boolean;
      updated_at: string;
    } | null;
  }> =>
    fetch(`${API_BASE}/agents/${encodeURIComponent(agentId)}/last-usage`, {
      headers: { 'X-API-Key': apiKey },
    }).then((r) => {
      if (!r.ok) throw new Error(r.statusText);
      return r.json().then((b) => b.data);
    }),

  testProviderConnection: (
    providerId: string,
    apiKey: string,
  ): Promise<{
    status: 'ok' | 'auth_failed' | 'unreachable' | 'model_list_unavailable';
    latency_ms: number;
    reachable: boolean;
    auth_ok: boolean;
    model_list: boolean;
    models_count: number | null;
    error: string | null;
  }> =>
    mutate(`/llm/providers/${encodeURIComponent(providerId)}/test`, 'POST', 'test provider connection', undefined, {
      'X-API-Key': apiKey,
    })
      .then((r) => r.json())
      .then((b) => b.data),

  setLlmProviderContextLength: (providerId: string, apiKey: string, contextLength: number | null) =>
    mutate(
      `/llm/providers/${encodeURIComponent(providerId)}/context-length`,
      'POST',
      'set provider context length',
      { context_length: contextLength },
      { 'X-API-Key': apiKey },
    ).then(() => {}),

  setLlmProviderThinkingMode: (providerId: string, apiKey: string, value: 'auto' | 'on' | 'off') =>
    mutate(
      `/llm/providers/${encodeURIComponent(providerId)}/thinking-mode`,
      'POST',
      'set provider thinking mode',
      { value },
      { 'X-API-Key': apiKey },
    ).then(() => {}),

  listProviderModels: (
    providerId: string,
    apiKey: string,
  ): Promise<{
    models: Array<{
      id: string;
      name?: string;
      loaded?: boolean;
      max_context_length?: number;
      loaded_context_length?: number;
      architecture?: string;
    }>;
    error_code?: string;
    error?: string;
  }> =>
    fetch(`${API_BASE}/llm/providers/${encodeURIComponent(providerId)}/models`, {
      headers: { 'X-API-Key': apiKey },
    }).then((r) => {
      if (!r.ok) throw new Error(r.statusText);
      return r.json().then((b) => b.data);
    }),

  // Avatar Management
  async uploadAvatar(agentId: string, file: File, apiKey: string): Promise<{ avatar_description?: string }> {
    const res = await fetch(`${API_BASE}/agents/${encodeURIComponent(agentId)}/avatar`, {
      method: 'POST',
      headers: { 'Content-Type': file.type, 'X-API-Key': apiKey },
      body: file,
    });
    await throwIfNotOk(res, 'upload avatar');
    return res.json().then((b: { data: { avatar_description?: string } }) => b.data);
  },

  deleteAvatar: (agentId: string, apiKey: string) =>
    mutate(`/agents/${encodeURIComponent(agentId)}/avatar`, 'DELETE', 'delete avatar', undefined, {
      'X-API-Key': apiKey,
    }).then(() => {}),

  getAvatarUrl(agentId: string): string {
    return `${API_BASE}/agents/${encodeURIComponent(agentId)}/avatar`;
  },

  // VRM Model Management
  async uploadVrm(agentId: string, file: File, apiKey: string): Promise<{ vrm_path?: string }> {
    const res = await fetch(`${API_BASE}/agents/${encodeURIComponent(agentId)}/vrm`, {
      method: 'POST',
      headers: { 'Content-Type': 'model/gltf-binary', 'X-API-Key': apiKey },
      body: file,
    });
    await throwIfNotOk(res, 'upload VRM');
    return res.json().then((b: { data: { vrm_path?: string } }) => b.data);
  },

  deleteVrm: (agentId: string, apiKey: string) =>
    mutate(`/agents/${encodeURIComponent(agentId)}/vrm`, 'DELETE', 'delete VRM', undefined, {
      'X-API-Key': apiKey,
    }).then(() => {}),

  getVrmUrl(agentId: string): string {
    return `${API_BASE}/agents/${encodeURIComponent(agentId)}/vrm`;
  },

  // Viseme Generation
  async generateVisemes(
    agentId: string,
    text: string,
    apiKey?: string,
  ): Promise<{ entries: Array<{ viseme: string; start_ms: number; duration_ms: number }>; total_duration_ms: number }> {
    const headers: Record<string, string> = { 'Content-Type': 'application/json' };
    if (apiKey) headers['X-API-Key'] = apiKey;
    const res = await fetch(`${API_BASE}/agents/${encodeURIComponent(agentId)}/visemes`, {
      method: 'POST',
      headers,
      body: JSON.stringify({ text }),
    });
    await throwIfNotOk(res, 'generate visemes');
    return res.json().then(
      (b: {
        data: {
          entries: Array<{ viseme: string; start_ms: number; duration_ms: number }>;
          total_duration_ms: number;
        };
      }) => b.data,
    );
  },

  // Bootstrap Setup
  getSetupStatus: (): Promise<SetupStatus> => fetchJson<SetupStatus>('/setup/status', 'fetch setup status'),

  startSetup: (apiKey: string): Promise<void> =>
    mutate('/setup/start', 'POST', 'start setup', undefined, { 'X-API-Key': apiKey }).then(() => {}),

  getSetupProgressUrl: (): string => `${API_BASE}/setup/progress`,

  // Marketplace
  getMarketplaceCatalog: async (
    apiKey: string,
    forceRefresh = false,
  ): Promise<{ servers: MarketplaceCatalogEntry[]; cached_at: string }> => {
    const url = forceRefresh ? `${API_BASE}/marketplace/catalog?force_refresh=true` : `${API_BASE}/marketplace/catalog`;
    const res = await fetch(url, { headers: { 'X-API-Key': apiKey } });
    if (!res.ok) throw new Error(`Marketplace catalog: ${res.status}`);
    return res.json().then((b: { data: { servers: MarketplaceCatalogEntry[]; cached_at: string } }) => b.data);
  },

  installMarketplaceServer: (
    payload: { server_id: string; env?: Record<string, string>; auto_start?: boolean },
    apiKey: string,
  ) =>
    mutate('/marketplace/install', 'POST', 'install marketplace server', payload, { 'X-API-Key': apiKey })
      .then((r) => r.json())
      .then((b) => b.data),

  getMarketplaceProgressUrl: (): string => `${API_BASE}/marketplace/progress`,

  // Health check
  scanHealth: (apiKey: string, fresh?: boolean) =>
    fetchJson<HealthReport>(`/health/scan${fresh ? '?fresh=true' : ''}`, 'scan health', apiKey),
  repairHealth: (apiKey: string) =>
    mutate('/health/repair', 'POST', 'repair health', undefined, { 'X-API-Key': apiKey })
      .then((r) => r.json())
      .then((b) => b.data as RepairReport),

  batchInstallMarketplaceServers: (payload: { server_ids: string[]; auto_start?: boolean }, apiKey: string) =>
    mutate('/marketplace/batch-install', 'POST', 'batch install marketplace servers', payload, { 'X-API-Key': apiKey })
      .then((r) => r.json())
      .then((b) => b.data),

  uninstallMarketplaceServer: (serverId: string, apiKey: string) =>
    mutate(
      `/marketplace/servers/${encodeURIComponent(serverId)}`,
      'DELETE',
      'uninstall marketplace server',
      undefined,
      { 'X-API-Key': apiKey },
    )
      .then((r) => r.json())
      .then((b) => b.data),

  // Memory Management
  deleteMemory: (memoryId: number, apiKey: string) =>
    mutate(`/memories/${memoryId}`, 'DELETE', 'delete memory', undefined, { 'X-API-Key': apiKey }).then(() => {}),

  updateMemory: (memoryId: number, content: string, apiKey: string) =>
    mutate(`/memories/${memoryId}`, 'PUT', 'update memory', { content }, { 'X-API-Key': apiKey }).then(() => {}),

  lockMemory: async (memoryId: number, apiKey: string): Promise<{ lock_level: string }> => {
    const res = await mutate(`/memories/${memoryId}/lock`, 'POST', 'lock memory', undefined, {
      'X-API-Key': apiKey,
    });
    const body = await res.json();
    return body.data as { lock_level: string };
  },

  unlockMemory: async (memoryId: number, apiKey: string): Promise<{ lock_level: string }> => {
    const res = await mutate(`/memories/${memoryId}/unlock`, 'POST', 'unlock memory', undefined, {
      'X-API-Key': apiKey,
    });
    const body = await res.json();
    return body.data as { lock_level: string };
  },

  deleteEpisode: (episodeId: number, apiKey: string) =>
    mutate(`/episodes/${episodeId}`, 'DELETE', 'delete episode', undefined, { 'X-API-Key': apiKey }).then(() => {}),

  importMemories: async (data: string, agentId: string, apiKey: string): Promise<ImportMemoriesResult> => {
    const res = await mutate(
      '/memories/import',
      'POST',
      'import memories',
      { data, agent_id: agentId },
      {
        'X-API-Key': apiKey,
      },
    );
    const body = await res.json();
    return body.data as ImportMemoriesResult;
  },
};

export interface ImportMemoriesResult {
  ok: boolean;
  dry_run: boolean;
  imported_memories: number;
  skipped_memories: number;
  imported_episodes: number;
  profile_updated: boolean;
  errors?: string[];
}

/** Pre-bind apiKey to all API methods, eliminating repetitive key passing. */
export function createAuthenticatedApi(apiKey: string) {
  const k = apiKey;
  return {
    // Pass-through (no apiKey needed)
    getHealth: () => api.getHealth(),
    getVersion: () => api.getVersion(),
    getAttachmentUrl: (id: string) => api.getAttachmentUrl(id),
    getAvatarUrl: (id: string) => api.getAvatarUrl(id),
    // Read
    getAgents: () => api.getAgents(k),
    getPendingPermissions: () => api.getPendingPermissions(k),
    getMetrics: () => api.getMetrics(k),
    getMemories: () => api.getMemories(k),
    getEpisodes: () => api.getEpisodes(k),
    getHistory: () => api.getHistory(k),
    getPlugins: () => api.getPlugins(k),
    getAgentAccess: (agentId: string) => api.getAgentAccess(agentId, k),
    // Generic
    fetchJson: <T>(path: string) => api.fetchJson<T>(path, k),
    put: (path: string, body: unknown) => api.put(path, body, k),
    post: (path: string, payload: unknown) => api.post(path, payload, k),
    // Agent CRUD
    createAgent: (payload: Parameters<typeof api.createAgent>[0]) => api.createAgent(payload, k),
    updateAgent: (id: string, payload: Parameters<typeof api.updateAgent>[1]) => api.updateAgent(id, payload, k),
    deleteAgent: (agentId: string, password?: string) => api.deleteAgent(agentId, k, password),
    toggleAgentPower: (agentId: string, enabled: boolean, password?: string) =>
      api.toggleAgentPower(agentId, enabled, k, password),
    // Chat
    postChat: (message: Parameters<typeof api.postChat>[0]) => api.postChat(message, k),
    postChatMessage: (agentId: string, msg: Parameters<typeof api.postChatMessage>[1]) =>
      api.postChatMessage(agentId, msg, k),
    getChatMessages: (agentId: string, before?: number, limit?: number, userId?: string) =>
      api.getChatMessages(agentId, k, before, limit, userId),
    deleteChatMessages: (agentId: string, userId?: string) => api.deleteChatMessages(agentId, k, userId),
    retryResponse: (agentId: string, messageId: string) => api.retryResponse(agentId, messageId, k),
    // Permissions
    getPluginPermissions: (pluginId: string) => api.getPluginPermissions(pluginId, k),
    revokePermission: (pluginId: string, permission: string) => api.revokePermission(pluginId, permission, k),
    grantPermission: (pluginId: string, permission: string) => api.grantPermission(pluginId, permission, k),
    approvePermission: (requestId: string, approvedBy: string) => api.approvePermission(requestId, approvedBy, k),
    denyPermission: (requestId: string, approvedBy: string) => api.denyPermission(requestId, approvedBy, k),
    // Events
    postEvent: (eventData: unknown) => api.postEvent(eventData, k),
    // Command approval
    approveCommand: (approvalId: string) => api.approveCommand(approvalId, k),
    trustCommand: (approvalId: string) => api.trustCommand(approvalId, k),
    denyCommand: (approvalId: string) => api.denyCommand(approvalId, k),
    // System
    invalidateApiKey: () => api.invalidateApiKey(k),
    // MCP servers
    listMcpServers: () => api.listMcpServers(k),
    getMcpServerSettings: (name: string) => api.getMcpServerSettings(name, k),
    updateMcpServerSettings: (name: string, settings: Parameters<typeof api.updateMcpServerSettings>[1]) =>
      api.updateMcpServerSettings(name, settings, k),
    getMcpServerAccess: (name: string) => api.getMcpServerAccess(name, k),
    putMcpServerAccess: (name: string, entries: Parameters<typeof api.putMcpServerAccess>[1]) =>
      api.putMcpServerAccess(name, entries, k),
    putAgentMcpAccess: (agentId: string, grantedServerIds: string[]) =>
      api.putAgentMcpAccess(agentId, grantedServerIds, k),
    startMcpServer: (name: string) => api.startMcpServer(name, k),
    stopMcpServer: (name: string) => api.stopMcpServer(name, k),
    restartMcpServer: (name: string) => api.restartMcpServer(name, k),
    createMcpServer: (payload: Parameters<typeof api.createMcpServer>[0]) => api.createMcpServer(payload, k),
    deleteMcpServer: (name: string) => api.deleteMcpServer(name, k),
    // Cron jobs
    listCronJobs: (agentId?: string) => api.listCronJobs(k, agentId),
    createCronJob: (payload: Parameters<typeof api.createCronJob>[0]) => api.createCronJob(payload, k),
    deleteCronJob: (jobId: string) => api.deleteCronJob(jobId, k),
    toggleCronJob: (jobId: string, enabled: boolean) => api.toggleCronJob(jobId, enabled, k),
    runCronJobNow: (jobId: string) => api.runCronJobNow(jobId, k),
    // LLM providers
    listLlmProviders: () => api.listLlmProviders(k),
    setLlmProviderKey: (providerId: string, providerApiKey: string) =>
      api.setLlmProviderKey(providerId, k, providerApiKey),
    deleteLlmProviderKey: (providerId: string) => api.deleteLlmProviderKey(providerId, k),
    setLlmProviderModel: (providerId: string, modelId: string) => api.setLlmProviderModel(providerId, k, modelId),
    setLlmProviderContextLength: (providerId: string, contextLength: number | null) =>
      api.setLlmProviderContextLength(providerId, k, contextLength),
    setLlmProviderThinkingMode: (providerId: string, value: 'auto' | 'on' | 'off') =>
      api.setLlmProviderThinkingMode(providerId, k, value),
    listProviderModels: (providerId: string) => api.listProviderModels(providerId, k),
    testProviderConnection: (providerId: string) => api.testProviderConnection(providerId, k),
    getAgentLastUsage: (agentId: string) => api.getAgentLastUsage(agentId, k),
    // Avatar
    uploadAvatar: (agentId: string, file: File) => api.uploadAvatar(agentId, file, k),
    deleteAvatar: (agentId: string) => api.deleteAvatar(agentId, k),
    // VRM
    uploadVrm: (agentId: string, file: File) => api.uploadVrm(agentId, file, k),
    deleteVrm: (agentId: string) => api.deleteVrm(agentId, k),
    getVrmUrl: (agentId: string) => api.getVrmUrl(agentId),
    // Memory
    deleteMemory: (memoryId: number) => api.deleteMemory(memoryId, k),
    updateMemory: (memoryId: number, content: string) => api.updateMemory(memoryId, content, k),
    lockMemory: (memoryId: number) => api.lockMemory(memoryId, k),
    unlockMemory: (memoryId: number) => api.unlockMemory(memoryId, k),
    deleteEpisode: (episodeId: number) => api.deleteEpisode(episodeId, k),
    importMemories: (data: string, agentId: string) => api.importMemories(data, agentId, k),
    // Setup
    getSetupStatus: () => api.getSetupStatus(),
    startSetup: () => api.startSetup(k),
    getSetupProgressUrl: () => api.getSetupProgressUrl(),
    // Marketplace
    getMarketplaceCatalog: (forceRefresh?: boolean) => api.getMarketplaceCatalog(k, forceRefresh),
    installMarketplaceServer: (payload: { server_id: string; env?: Record<string, string>; auto_start?: boolean }) =>
      api.installMarketplaceServer(payload, k),
    batchInstallMarketplaceServers: (payload: { server_ids: string[]; auto_start?: boolean }) =>
      api.batchInstallMarketplaceServers(payload, k),
    uninstallMarketplaceServer: (serverId: string) => api.uninstallMarketplaceServer(serverId, k),
    getMarketplaceProgressUrl: () => api.getMarketplaceProgressUrl(),
    scanHealth: (fresh?: boolean) => api.scanHealth(k, fresh),
    repairHealth: () => api.repairHealth(k),
  };
}

export type AuthenticatedApi = ReturnType<typeof createAuthenticatedApi>;
