import { AgentMetadata, PluginManifest } from '../types';

// Tauri WebView environment detection
const isTauri = '__TAURI_INTERNALS__' in window;

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
