export type ClotoId = string;

export interface ClotoMessage {
  id: string;
  source: { type: 'User'; id: string; name: string } | { type: 'Agent'; id: string } | { type: 'System' };
  target_agent?: string;
  content: string;
  timestamp: string;
  metadata: Record<string, string>;
}

export interface AgentMetadata {
  id: ClotoId;
  name: string;
  description: string;
  default_engine_id?: string;
  required_capabilities: CapabilityType[];
  enabled: boolean;
  last_seen: number;
  status: 'online' | 'offline' | 'degraded';
  metadata: Record<string, string>;
  agent_type?: 'agent' | 'system';
}

export type Permission =
  | 'VisionRead'
  | 'InputControl'
  | 'FileRead'
  | 'FileWrite'
  | 'NetworkAccess'
  | 'ProcessExecution'
  | 'MemoryRead'
  | 'MemoryWrite'
  | 'AdminAccess';

export type CapabilityType = 'Reasoning' | 'Memory' | 'Communication' | 'Tool' | 'Vision' | 'HAL' | 'Web';

// Event types for SSE stream and history
export interface StrictSystemEvent {
  type: string;
  timestamp: number;
  payload?: Record<string, unknown>;
  data?: unknown;
}

// Chat persistence types
export interface ContentBlock {
  type: 'text' | 'image' | 'code' | 'tool_result' | 'file' | 'audio';
  text?: string;
  url?: string;
  language?: string;
  filename?: string;
  mime_type?: string;
  attachment_id?: string;
  metadata?: Record<string, unknown>;
  duration?: number;
}

export interface ChatMessage {
  id: string;
  agent_id: string;
  user_id: string;
  source: 'user' | 'agent' | 'system';
  content: ContentBlock[];
  metadata?: Record<string, unknown>;
  created_at: number;
  parent_id?: string | null;
  branch_index?: number;
}

// API response types
export interface PermissionRequest {
  request_id: string;
  plugin_id: string;
  permission_type: string;
  target_resource?: string;
  justification: string;
  status: string;
  created_at: string;
}

export interface CommandApprovalRequest {
  approval_id: string;
  agent_id: string;
  commands: Array<{ command: string; command_name: string }>;
}

export interface Metrics {
  total_requests: number;
  total_memories: number;
  total_episodes: number;
  ram_usage: string;
}

export interface Memory {
  id: number;
  agent_id: string;
  content: string;
  source: Record<string, string>;
  timestamp: string;
  created_at: string;
  locked?: boolean;
  lock_level?: 'server' | 'kernel';
}

export interface MemoryCapabilities {
  update_memory: boolean;
  lock_memory: boolean;
  unlock_memory: boolean;
}

export interface Episode {
  id: number;
  agent_id: string;
  summary: string;
  keywords: string;
  start_time?: string;
  end_time?: string;
  created_at: string;
}

export interface InstalledConfig {
  pluginId: string;
  x: number;
  y: number;
}

// MCP Server Management types (MCP_SERVER_UI_DESIGN.md)
export type McpServerStatus = 'Connected' | 'Disconnected' | 'Error';
export type DefaultPolicy = 'opt-in' | 'opt-out';
export type EntryType = 'capability' | 'server_grant' | 'tool_grant';
export type AccessPermission = 'allow' | 'deny';

export interface McpServerInfo {
  id: string;
  command: string;
  args: string[];
  status: McpServerStatus;
  status_message?: string;
  tools: string[];
  is_cloto_sdk: boolean;
  display_name?: string;
  mgp_supported?: boolean;
  trust_level?: string;
  transport?: string;
  url?: string;
  has_unresolved_env?: boolean;
}

export interface AccessControlEntry {
  id?: number;
  entry_type: EntryType;
  agent_id: string;
  server_id: string;
  tool_name?: string;
  permission: AccessPermission;
  granted_by?: string;
  granted_at: string;
  expires_at?: string;
  justification?: string;
}

export interface AccessTreeResponse {
  server_id: string;
  default_policy: DefaultPolicy;
  tools: string[];
  entries: AccessControlEntry[];
}

export interface McpServerSettings {
  server_id: string;
  default_policy: DefaultPolicy;
  config: Record<string, string>;
  env?: Record<string, string>;
  auto_restart: boolean;
  command: string;
  args: string[];
  description?: string;
}

// Marketplace types
export type ServerCategory = 'mind' | 'memory' | 'tool' | 'vision' | 'voice' | 'output' | 'io';

export interface EnvVarDef {
  key: string;
  default?: string;
  description?: string;
}

export interface MarketplaceCatalogEntry {
  id: string;
  name: string;
  description: string;
  category: ServerCategory;
  version: string;
  directory: string;
  dependencies: string[];
  env_vars: EnvVarDef[];
  optional_env_vars: EnvVarDef[];
  tags: string[];
  trust_level: string;
  auto_restart: boolean;
  icon?: string;
  runtime: string;
  installed: boolean;
  installed_version?: string;
  update_available: boolean;
  running: boolean;
  changelog?: string;
}

// Bootstrap Setup types
export interface SetupStatus {
  setup_complete: boolean;
  mcp_servers_present: boolean;
  uv_available: boolean;
  venv_exists: boolean;
  setup_in_progress: boolean;
}

// Cron Job Scheduler (Layer 2: Autonomous Trigger)
export type ScheduleType = 'interval' | 'cron' | 'once';

export interface CronJob {
  id: string;
  agent_id: string;
  name: string;
  enabled: boolean;
  schedule_type: ScheduleType;
  schedule_value: string;
  engine_id?: string;
  message: string;
  next_run_at: number;
  last_run_at?: number;
  last_status?: 'success' | 'error';
  last_error?: string;
  max_iterations?: number;
  created_at: string;
  hide_prompt?: boolean;
  cron_generation?: number;
  source_type?: 'user' | 'system';
  creator_user_id?: string;
  creator_user_name?: string;
}

// ── Actions Panel: Inter-agent dialogue ──

export interface AgentDialogue {
  dialogue_id: string;
  caller_agent_id: string;
  caller_agent_name: string;
  target_agent_id: string;
  target_agent_name: string;
  prompt: string;
  engine_id: string;
  response: string | null;
  chain_depth: number;
  status: 'pending' | 'success' | 'error';
  timestamp: number;
}

export interface ExternalAction {
  action_id: string;
  source: string;
  source_label: string;
  target_agent_id: string;
  target_agent_name: string;
  prompt: string;
  sender_name: string;
  engine_id: string;
  response: string | null;
  status: 'pending' | 'success' | 'error';
  callback_id: string;
  timestamp: number;
}
