export type ExivId = string;

export interface ExivMessage {
  id: string;
  source: 
    | { type: 'User'; id: string; name: string }
    | { type: 'Agent'; id: string }
    | { type: 'System' };
  target_agent?: string;
  content: string;
  timestamp: string;
  metadata: Record<string, string>;
}

export interface AgentMetadata {
  id: ExivId;
  name: string;
  description: string;
  default_engine_id?: string;
  required_capabilities: CapabilityType[];
  enabled: boolean;
  last_seen: number;
  status: 'online' | 'offline' | 'degraded';
  metadata: Record<string, string>;
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

export type CapabilityType =
  | 'Reasoning'
  | 'Memory'
  | 'Communication'
  | 'Tool'
  | 'Vision'
  | 'HAL'
  | 'Web';

export type PluginCategory = 'Agent' | 'Tool' | 'Memory' | 'System' | 'Other';

export interface PluginManifest {
  id: ExivId;
  name: string;
  description: string;
  version: string;
  category: PluginCategory;
  service_type: ServiceType;
  tags: string[];
  is_active: boolean;
  is_configured: boolean;
  required_config_keys: string[];
  action_icon?: string;
  action_target?: string;
  magic_seal: number;
  sdk_version: string;
  required_permissions: Permission[];
  provided_capabilities: CapabilityType[];
  provided_tools: string[];
}

export type ServiceType = 
  | 'Communication'
  | 'Reasoning'
  | 'Skill'
  | 'Vision'
  | 'Action'
  | 'Memory'
  | 'HAL';


export interface GazeData {
  x: number;
  y: number;
  confidence: number;
  fixated: boolean;
}

export type TrackerStatus = 'loading' | 'requesting' | 'active' | 'denied' | 'error' | 'stopped';
export type DelegateMode = 'gpu' | 'cpu' | 'cpu-fallback';

// Event types for SSE stream and history
export interface StrictSystemEvent {
  type: string;
  timestamp: number;
  payload?: Record<string, unknown>;
  data?: unknown;
}

// Neural network visualization types
export interface Node {
  id: string;
  label: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  type: string;
  color: string;
  data: { status: string; lastActive: string; log: string };
}

export interface Edge {
  source: string;
  target: string;
  color: string;
}

export interface Pulse {
  edge: Edge;
  progress: number;
  speed: number;
  color: string;
}

export interface ModalState {
  nodeId: string;
  offsetX: number;
  offsetY: number;
  isDragging: boolean;
}

// Chat persistence types
export interface ContentBlock {
  type: 'text' | 'image' | 'code' | 'tool_result' | 'file';
  text?: string;
  url?: string;
  language?: string;
  filename?: string;
  mime_type?: string;
  attachment_id?: string;
  metadata?: Record<string, unknown>;
}

export interface ChatMessage {
  id: string;
  agent_id: string;
  user_id: string;
  source: 'user' | 'agent' | 'system';
  content: ContentBlock[];
  metadata?: Record<string, unknown>;
  created_at: number;
}

// Evolution types (E6)
export interface FitnessScores {
  cognitive: number;
  behavioral: number;
  safety: number;
  autonomy: number;
  meta_learning: number;
}

export interface FitnessWeights {
  cognitive: number;
  behavioral: number;
  safety: number;
  autonomy: number;
  meta_learning: number;
}

export interface EvolutionParams {
  alpha: number;
  beta: number;
  theta_min: number;
  gamma: number;
  min_interactions: number;
  weights: FitnessWeights;
}

export interface AgentSnapshot {
  active_plugins: string[];
  plugin_capabilities?: Record<string, string[]>;
  personality_hash: string;
  strategy_params: Record<string, unknown>;
}

export interface GenerationRecord {
  generation: number;
  trigger: 'Evolution' | 'Regression' | 'SafetyBreach' | 'Rebalance' | 'AutonomyUpgrade' | 'CapabilityGain' | string;
  timestamp: string;
  interactions_since_last: number;
  scores: FitnessScores;
  delta: Record<string, number>;
  fitness: number;
  fitness_delta: number;
  snapshot: AgentSnapshot;
}

export interface FitnessLogEntry {
  timestamp: string;
  interaction_count: number;
  scores: FitnessScores;
  fitness: number;
}

export interface RollbackRecord {
  timestamp: string;
  from_generation: number;
  to_generation: number;
  reason: string;
  rollback_count_to_target: number;
}

export interface GracePeriodState {
  active: boolean;
  started_at: string;
  interactions_at_start: number;
  grace_interactions: number;
  fitness_at_start: number;
  affected_axis: string;
}

export interface EvolutionStatus {
  agent_id: string;
  current_generation: number;
  fitness: number;
  scores: FitnessScores;
  trend: 'improving' | 'declining' | 'stable';
  interaction_count: number;
  interactions_since_last_gen: number;
  grace_period: GracePeriodState | null;
  autonomy_level: string; // e.g. "L0"-"L5"
  top_axes: [string, number][];
}

export interface EvolutionEvent {
  type: string;
  data: Record<string, unknown>;
  timestamp: number;
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

export interface Metrics {
  total_requests: number;
  total_memories: number;
  total_episodes: number;
  ram_usage: string;
}

export interface Memory {
  user_id: string;
  guild_id: string;
  content: string;
  updated_at: string;
}

export interface Episode {
  id: number;
  summary: string;
  start_time: string;
  channel_id?: string;
}

export interface InstalledConfig {
  pluginId: string;
  x: number;
  y: number;
}

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

// MCP Server Management types (MCP_SERVER_UI_DESIGN.md)
export type McpServerStatus = 'Connected' | 'Disconnected' | 'Error';
export type ServerSource = 'config' | 'dynamic';
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
  is_exiv_sdk: boolean;
  source: ServerSource;
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
  auto_restart: boolean;
  command: string;
  args: string[];
  description?: string;
}