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
  it('matches the legacy mind.* prefix', () => {
    expect(isEngineServer(server('mind.local'))).toBe(true);
  });

  it('matches de-prefixed catalog engines by the think tool surface', () => {
    // bug-388/396: ClotoHub registers `deepseek`, not `mind.deepseek`.
    expect(isEngineServer(server('deepseek', ['think', 'think_with_tools']))).toBe(true);
  });

  it('rejects non-engine servers', () => {
    expect(isEngineServer(server('cscheduler', ['create_task', 'list_goals']))).toBe(false);
    expect(isEngineServer(server('cpersona', ['store', 'recall']))).toBe(false);
  });
});

describe('isMemoryServer', () => {
  it('matches the legacy memory.* prefix', () => {
    expect(isMemoryServer(server('memory.cpersona'))).toBe(true);
  });

  it('matches de-prefixed catalog memory backends by the memory tool surface', () => {
    expect(isMemoryServer(server('cpersona', ['store', 'recall', 'list_memories']))).toBe(true);
  });

  it('rejects non-memory servers', () => {
    expect(isMemoryServer(server('mind.local', ['think']))).toBe(false);
    expect(isMemoryServer(server('websearch', ['search']))).toBe(false);
  });
});
