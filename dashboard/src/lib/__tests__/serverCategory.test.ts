import { describe, expect, it } from 'vitest';
import type { McpServerInfo } from '../../types';
import { isEngineServer, isMemoryServer } from '../serverCategory';

function server(id: string, tools: string[] = []): McpServerInfo {
  return {
    id,
    command: 'python',
    args: [],
    status: 'Connected',
    tools,
    is_cloto_sdk: true,
  };
}

describe('isEngineServer', () => {
  it('matches engines by the think tool surface', () => {
    expect(isEngineServer(server('deepseek', ['think', 'think_with_tools']))).toBe(true);
    expect(isEngineServer(server('local', ['think']))).toBe(true);
  });

  it('carries no id semantics — a dotted id without the tool surface is not an engine', () => {
    // Prefixes are retired; classification is tool-surface only.
    expect(isEngineServer(server('mind.local'))).toBe(false);
  });

  it('rejects non-engine servers', () => {
    expect(isEngineServer(server('cscheduler', ['create_task', 'list_goals']))).toBe(false);
    expect(isEngineServer(server('cpersona', ['store', 'recall']))).toBe(false);
  });
});

describe('isMemoryServer', () => {
  it('matches memory backends by the memory tool surface', () => {
    expect(isMemoryServer(server('cpersona', ['store', 'recall', 'list_memories']))).toBe(true);
  });

  it('carries no id semantics — a dotted id without the tool surface is not memory', () => {
    expect(isMemoryServer(server('memory.cpersona'))).toBe(false);
  });

  it('rejects non-memory servers', () => {
    expect(isMemoryServer(server('local', ['think']))).toBe(false);
    expect(isMemoryServer(server('websearch', ['search']))).toBe(false);
  });
});
