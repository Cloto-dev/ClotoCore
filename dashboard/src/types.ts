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
  status: 'online' | 'offline' | 'busy';
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
  | 'MemoryWrite';

export type CapabilityType =
  | 'Reasoning'
  | 'Memory'
  | 'Communication'
  | 'Tool'
  | 'Vision'
  | 'HAL';

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

export interface ColorVisionData {
  captured_at: string;
  detected_elements: DetectedElement[];
}

export interface DetectedElement {
  label: string;
  bounds: [number, number, number, number];
  confidence: number;
}

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
  node: Node | null;
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