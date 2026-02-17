import { AgentMetadata, PluginManifest, ContentBlock, ChatMessage } from '../types';
import { isTauri } from '../lib/tauri';

// In Tauri mode, window.location.origin returns "tauri://localhost" which cannot reach
// the HTTP kernel. We must use the actual loopback address with the kernel port.
const KERNEL_PORT = 8081;
const API_URL = import.meta.env.VITE_API_URL
  || (isTauri ? `http://127.0.0.1:${KERNEL_PORT}/api` : `${window.location.origin}/api`);
export const API_BASE = API_URL.endsWith('/api') ? API_URL : `${API_URL}/api`;

export const api = {
  async getAgents(): Promise<AgentMetadata[]> {
    const res = await fetch(`${API_BASE}/agents`);
    if (!res.ok) throw new Error(`Failed to fetch agents: ${res.statusText}`);
    return res.json();
  },

  async getPlugins(): Promise<PluginManifest[]> {
    const res = await fetch(`${API_BASE}/plugins`);
    if (!res.ok) throw new Error(`Failed to fetch plugins: ${res.statusText}`);
    return res.json();
  },

  async applyPluginSettings(settings: { id: string, is_active: boolean }[]): Promise<void> {
    const res = await fetch(`${API_BASE}/plugins/apply`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(settings)
    });
    if (!res.ok) throw new Error(`Failed to apply plugin settings: ${res.statusText}`);
  },

  async getPluginConfig(id: string): Promise<Record<string, string>> {
    const res = await fetch(`${API_BASE}/plugins/${id}/config`);
    if (!res.ok) throw new Error(`Failed to get plugin config: ${res.statusText}`);
    return res.json();
  },

  async updatePluginConfig(id: string, payload: { key: string, value: string }): Promise<void> {
    const res = await fetch(`${API_BASE}/plugins/${id}/config`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload)
    });
    if (!res.ok) throw new Error(`Failed to update plugin config: ${res.statusText}`);
  },

  async updateAgent(id: string, payload: { default_engine_id?: string, metadata: Record<string, string> }): Promise<void> {
    const res = await fetch(`${API_BASE}/agents/${id}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload)
    });
    if (!res.ok) throw new Error(`Failed to update agent: ${res.statusText}`);
  },

  async toggleAgentPower(agentId: string, enabled: boolean, password?: string): Promise<void> {
    const res = await fetch(`${API_BASE}/agents/${agentId}/power`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ enabled, password: password || undefined })
    });
    if (!res.ok) {
      const body = await res.json().catch(() => ({}));
      throw new Error(body?.error?.message || `Failed to toggle agent power: ${res.statusText}`);
    }
  },

  async grantPermission(pluginId: string, permission: string): Promise<void> {
    const res = await fetch(`${API_BASE}/plugins/${pluginId}/permissions/grant`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ permission })
    });
    if (!res.ok) throw new Error(`Failed to grant permission: ${res.statusText}`);
  },

  async postEvent(eventData: any): Promise<void> {
    const res = await fetch(`${API_BASE}/events/publish`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(eventData)
    });
    if (!res.ok) throw new Error(`Failed to post event: ${res.statusText}`);
  },

  async post(path: string, payload: any): Promise<void> {
    const res = await fetch(`${API_BASE}${path}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload)
    });
    if (!res.ok) throw new Error(`Failed to post to ${path}: ${res.statusText}`);
  },

  async getPendingPermissions(): Promise<any[]> {
    const res = await fetch(`${API_BASE}/permissions/pending`);
    if (!res.ok) throw new Error(`Failed to fetch pending permissions: ${res.statusText}`);
    return res.json();
  },

  async approvePermission(requestId: string, approvedBy: string): Promise<void> {
    const res = await fetch(`${API_BASE}/permissions/${requestId}/approve`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ approved_by: approvedBy })
    });
    if (!res.ok) throw new Error(`Failed to approve permission: ${res.statusText}`);
  },

  async denyPermission(requestId: string, approvedBy: string): Promise<void> {
    const res = await fetch(`${API_BASE}/permissions/${requestId}/deny`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ approved_by: approvedBy })
    });
    if (!res.ok) throw new Error(`Failed to deny permission: ${res.statusText}`);
  },

  async checkForUpdate(): Promise<UpdateInfo> {
    const res = await fetch(`${API_BASE}/system/update/check`);
    if (!res.ok) throw new Error(`Failed to check for updates: ${res.statusText}`);
    return res.json();
  },

  async applyUpdate(version: string): Promise<UpdateResult> {
    const res = await fetch(`${API_BASE}/system/update/apply`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ version })
    });
    if (!res.ok) throw new Error(`Failed to apply update: ${res.statusText}`);
    return res.json();
  },

  async getVersion(): Promise<{ version: string; build_target: string }> {
    const res = await fetch(`${API_BASE}/system/version`);
    if (!res.ok) throw new Error(`Failed to fetch version: ${res.statusText}`);
    return res.json();
  },

  // Chat persistence API
  async getChatMessages(agentId: string, before?: number, limit?: number): Promise<{ messages: ChatMessage[], has_more: boolean }> {
    const params = new URLSearchParams();
    if (before) params.set('before', String(before));
    if (limit) params.set('limit', String(limit));
    const qs = params.toString();
    const res = await fetch(`${API_BASE}/chat/${agentId}/messages${qs ? '?' + qs : ''}`);
    if (!res.ok) throw new Error(`Failed to fetch chat messages: ${res.statusText}`);
    const data = await res.json();
    // Parse content from JSON string to ContentBlock[]
    return {
      messages: data.messages.map((m: any) => ({
        ...m,
        content: typeof m.content === 'string' ? JSON.parse(m.content) : m.content,
        metadata: m.metadata ? (typeof m.metadata === 'string' ? JSON.parse(m.metadata) : m.metadata) : undefined,
      })),
      has_more: data.has_more,
    };
  },

  async postChatMessage(agentId: string, msg: { id: string; source: string; content: ContentBlock[]; metadata?: Record<string, unknown> }): Promise<{ id: string; created_at: number }> {
    const res = await fetch(`${API_BASE}/chat/${agentId}/messages`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(msg),
    });
    if (!res.ok) throw new Error(`Failed to post chat message: ${res.statusText}`);
    return res.json();
  },

  async deleteChatMessages(agentId: string): Promise<{ deleted_count: number }> {
    const res = await fetch(`${API_BASE}/chat/${agentId}/messages`, {
      method: 'DELETE',
    });
    if (!res.ok) throw new Error(`Failed to delete chat messages: ${res.statusText}`);
    return res.json();
  },

  getAttachmentUrl(attachmentId: string): string {
    return `${API_BASE}/chat/attachments/${attachmentId}`;
  },

  async postChat(message: ExivMessage): Promise<void> {
    const res = await fetch(`${API_BASE}/chat`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(message)
    });
    if (!res.ok) throw new Error(`Chat request failed: ${res.status}`);
  },

  async createAgent(payload: {
    name: string;
    description: string;
    default_engine: string;
    metadata: Record<string, string>;
    password?: string;
  }): Promise<void> {
    const res = await fetch(`${API_BASE}/agents`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload)
    });
    if (!res.ok) throw new Error(`Failed to create agent: ${res.statusText}`);
  },

  async getMetrics(): Promise<any> {
    const res = await fetch(`${API_BASE}/metrics`);
    if (!res.ok) throw new Error(`Failed to fetch metrics: ${res.statusText}`);
    return res.json();
  },

  async getMemories(): Promise<any[]> {
    const res = await fetch(`${API_BASE}/memories`);
    if (!res.ok) throw new Error(`Failed to fetch memories: ${res.statusText}`);
    return res.json();
  },

  async getEpisodes(): Promise<any[]> {
    const res = await fetch(`${API_BASE}/episodes`);
    if (!res.ok) throw new Error(`Failed to fetch episodes: ${res.statusText}`);
    return res.json();
  },

  async getHistory(): Promise<any[]> {
    const res = await fetch(`${API_BASE}/history`);
    if (!res.ok) throw new Error(`Failed to fetch history: ${res.statusText}`);
    return res.json();
  }
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
