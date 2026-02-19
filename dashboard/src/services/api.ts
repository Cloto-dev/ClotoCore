import { AgentMetadata, PluginManifest, ContentBlock, ChatMessage, ExivMessage, EvolutionStatus, GenerationRecord, FitnessLogEntry, EvolutionParams, RollbackRecord } from '../types';
import { isTauri } from '../lib/tauri';

// In Tauri mode, window.location.origin returns "tauri://localhost" which cannot reach
// the HTTP kernel. We must use the actual loopback address with the kernel port.
const KERNEL_PORT = 8081;
const API_URL = import.meta.env.VITE_API_URL
  || (isTauri ? `http://127.0.0.1:${KERNEL_PORT}/api` : `${window.location.origin}/api`);
export const API_BASE = API_URL.endsWith('/api') ? API_URL : `${API_URL}/api`;

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
  getPendingPermissions: () => fetchJson<any[]>('/permissions/pending', 'fetch pending permissions'),
  checkForUpdate: () => fetchJson<UpdateInfo>('/system/update/check', 'check for updates'),
  getVersion: () => fetchJson<{ version: string; build_target: string }>('/system/version', 'fetch version'),
  getMetrics: () => fetchJson<any>('/metrics', 'fetch metrics'),
  getMemories: () => fetchJson<any[]>('/memories', 'fetch memories'),
  getEpisodes: () => fetchJson<any[]>('/episodes', 'fetch episodes'),
  getHistory: () => fetchJson<any[]>('/history', 'fetch history'),
  getEvolutionStatus: () => fetchJson<EvolutionStatus>('/evolution/status', 'fetch evolution status'),
  getGeneration: (n: number) => fetchJson<GenerationRecord>(`/evolution/generations/${n}`, 'fetch generation'),
  getEvolutionParams: () => fetchJson<EvolutionParams>('/evolution/params', 'fetch evolution params'),
  getRollbackHistory: () => fetchJson<RollbackRecord[]>('/evolution/rollbacks', 'fetch rollback history'),

  getGenerationHistory: (limit?: number) =>
    fetchJson<GenerationRecord[]>(`/evolution/generations${limit ? `?limit=${limit}` : ''}`, 'fetch generations'),
  getFitnessTimeline: (limit?: number) =>
    fetchJson<FitnessLogEntry[]>(`/evolution/fitness${limit ? `?limit=${limit}` : ''}`, 'fetch fitness timeline'),

  applyPluginSettings: (settings: { id: string, is_active: boolean }[], apiKey: string) =>
    mutate('/plugins/apply', 'POST', 'apply plugin settings', settings, { 'X-API-Key': apiKey }).then(() => {}),
  updatePluginConfig: (id: string, payload: { key: string, value: string }, apiKey: string) =>
    mutate(`/plugins/${id}/config`, 'POST', 'update plugin config', payload, { 'X-API-Key': apiKey }).then(() => {}),
  updateAgent: (id: string, payload: { default_engine_id?: string, metadata: Record<string, string> }, apiKey: string) =>
    mutate(`/agents/${id}`, 'POST', 'update agent', payload, { 'X-API-Key': apiKey }).then(() => {}),
  grantPermission: (pluginId: string, permission: string, apiKey: string) =>
    mutate(`/plugins/${pluginId}/permissions/grant`, 'POST', 'grant permission', { permission }, { 'X-API-Key': apiKey }).then(() => {}),
  postEvent: (eventData: any, apiKey: string) =>
    mutate('/events/publish', 'POST', 'post event', eventData, { 'X-API-Key': apiKey }).then(() => {}),
  post: (path: string, payload: any, apiKey: string) =>
    mutate(path, 'POST', `post to ${path}`, payload, { 'X-API-Key': apiKey }).then(() => {}),
  approvePermission: (requestId: string, approvedBy: string, apiKey: string) =>
    mutate(`/permissions/${requestId}/approve`, 'POST', 'approve permission', { approved_by: approvedBy }, { 'X-API-Key': apiKey }).then(() => {}),
  denyPermission: (requestId: string, approvedBy: string, apiKey: string) =>
    mutate(`/permissions/${requestId}/deny`, 'POST', 'deny permission', { approved_by: approvedBy }, { 'X-API-Key': apiKey }).then(() => {}),
  async createAgent(payload: { name: string; description: string; default_engine: string; metadata: Record<string, string>; password?: string }, apiKey: string): Promise<void> {
    const res = await fetch(`${API_BASE}/agents`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
      body: JSON.stringify(payload),
    });
    if (!res.ok) {
      const body = await res.json().catch(() => ({}));
      throw new Error(body?.error?.message || `Failed to create agent: ${res.statusText}`);
    }
  },
  postChat: (message: ExivMessage, apiKey: string) =>
    mutate('/chat', 'POST', 'send chat', message, { 'X-API-Key': apiKey }).then(() => {}),
  updateEvolutionParams: (params: EvolutionParams, apiKey: string) =>
    mutate('/evolution/params', 'PUT', 'update evolution params', params, { 'X-API-Key': apiKey }).then(() => {}),

  applyUpdate: (version: string, apiKey: string): Promise<UpdateResult> =>
    mutate('/system/update/apply', 'POST', 'apply update', { version }, { 'X-API-Key': apiKey }).then(r => r.json()),
  postChatMessage: (agentId: string, msg: { id: string; source: string; content: ContentBlock[]; metadata?: Record<string, unknown> }, apiKey: string): Promise<{ id: string; created_at: number }> =>
    mutate(`/chat/${agentId}/messages`, 'POST', 'post chat message', msg, { 'X-API-Key': apiKey }).then(r => r.json()),
  evaluateAgent: (scores: { cognitive: number; behavioral: number; safety: number; autonomy: number; meta_learning: number }, apiKey: string): Promise<{ status: string; events: unknown[] }> =>
    mutate('/evolution/evaluate', 'POST', 'evaluate', { scores }, { 'X-API-Key': apiKey }).then(r => r.json()),
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
    if (!res.ok) {
      const body = await res.json().catch(() => ({}));
      throw new Error(body?.error?.message || `Failed to toggle agent power: ${res.statusText}`);
    }
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
};

export interface UpdateInfo {
  current_version: string;
  latest_version?: string;
  update_available: boolean;
  release_url?: string;
  release_name?: string;
  release_notes?: string;
  published_at?: string;
  build_target?: string;
  message?: string;
  assets?: { name: string; size: number; download_url: string }[];
}

export interface UpdateResult {
  status: string;
  previous_version: string;
  new_version: string;
  sha256: string;
  message: string;
}
