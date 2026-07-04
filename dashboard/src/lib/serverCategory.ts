import type { McpServerInfo } from '../types';

// Capability classification for connected MCP servers, mirroring the kernel's
// `classify_tool` (crates/core/src/managers/capability_dispatcher.rs).
//
// Classification is tool-surface only. Server ids are bare and carry no
// category semantics — the legacy `mind.` / `memory.` prefix arms were
// retired with the prefixed ids themselves (an earlier decision,
// docs/CATEGORY_PREFIX_RETIREMENT_DESIGN.md).

const REASONING_TOOLS = ['think', 'think_with_tools'];

const MEMORY_TOOLS = new Set([
  'store',
  'recall',
  'list_memories',
  'delete_memory',
  'list_episodes',
  'delete_episode',
  'archive_episode',
  'delete_agent_data',
  'update_profile',
  'update_memory',
  'lock_memory',
  'unlock_memory',
  'set_recall_precision',
  'get_recall_precision',
]);

/** True when the server is a reasoning engine (exposes the `think` tool surface). */
export function isEngineServer(s: McpServerInfo): boolean {
  return s.tools.some((t) => REASONING_TOOLS.includes(t));
}

/** True when the server is a memory backend (exposes the memory tool surface). */
export function isMemoryServer(s: McpServerInfo): boolean {
  return s.tools.some((t) => MEMORY_TOOLS.has(t));
}
