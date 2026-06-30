import type { McpServerInfo } from '../types';

// Capability classification for connected MCP servers, mirroring the kernel's
// `classify_tool` (crates/core/src/managers/capability_dispatcher.rs).
//
// Servers installed via the ClotoHub catalog carry de-prefixed ids (e.g.
// `deepseek`, `cpersona`) instead of the legacy `mind.` / `memory.` namespace
// (bug-388 / bug-396). Prefix-only matching therefore misses every catalog
// connector and only catches built-ins like `mind.local`. Fall back to the
// same tool surface the kernel keys on so de-prefixed engines and memory
// backends are categorised correctly.

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

/** True when the server is a reasoning engine (legacy `mind.` prefix or a
 *  de-prefixed catalog engine exposing the `think` tool surface). */
export function isEngineServer(s: McpServerInfo): boolean {
  return s.id.startsWith('mind.') || s.tools.some((t) => REASONING_TOOLS.includes(t));
}

/** True when the server is a memory backend (legacy `memory.` prefix or a
 *  de-prefixed catalog backend exposing the memory tool surface). */
export function isMemoryServer(s: McpServerInfo): boolean {
  return s.id.startsWith('memory.') || s.tools.some((t) => MEMORY_TOOLS.has(t));
}
