import { describe, expect, it } from 'vitest';
import type { AccessControlEntry } from '../../types';
import {
  changedServers,
  deniedToolCount,
  grantsFor,
  grantsFromEntries,
  mergeAgentEntries,
  withServerGrant,
  withToolGrant,
} from '../agentAccess';

function entry(over: Partial<AccessControlEntry> = {}): AccessControlEntry {
  return {
    entry_type: 'server_grant',
    agent_id: 'agent.a',
    server_id: 'cpersona',
    permission: 'allow',
    granted_by: 'user',
    granted_at: '2026-09-17T00:00:00Z',
    ...over,
  };
}

describe("one agent's grants", () => {
  it('reads the server answer and the per-tool exceptions, and ignores other agents', () => {
    const grants = grantsFromEntries(
      [
        entry(),
        entry({ entry_type: 'tool_grant', tool_name: 'delete_memory', permission: 'deny' }),
        entry({ agent_id: 'agent.other', permission: 'deny' }),
      ],
      'agent.a',
    );
    expect(grantsFor(grants, 'cpersona').server).toBe('allow');
    expect(grantsFor(grants, 'cpersona').tools).toEqual({ delete_memory: 'deny' });
    expect(deniedToolCount(grants, 'cpersona')).toBe(1);
  });

  it('treats a server with no entry as default, and clearing a tool as removing its entry', () => {
    let grants = {};
    expect(grantsFor(grants, 'nowhere').server).toBe('default');

    grants = withToolGrant(grants, 'cpersona', 'recall', 'deny');
    expect(deniedToolCount(grants, 'cpersona')).toBe(1);
    grants = withToolGrant(grants, 'cpersona', 'recall', 'default');
    expect(grantsFor(grants, 'cpersona').tools).toEqual({});
    expect(deniedToolCount(grants, 'cpersona')).toBe(0);
  });

  it('names exactly the servers whose answer moved', () => {
    const before = withServerGrant({}, 'cpersona', 'allow');
    const after = withToolGrant(withServerGrant(before, 'websearch', 'deny'), 'cpersona', 'recall', 'deny');
    expect(changedServers(before, after).sort()).toEqual(['cpersona', 'websearch']);
    expect(changedServers(before, before)).toEqual([]);
    // A different permission on the same server is a change, not a no-op.
    expect(changedServers(before, withServerGrant(before, 'cpersona', 'deny'))).toEqual(['cpersona']);
  });
});

describe('the list written back to a server', () => {
  it("keeps every other agent's entries — the endpoint replaces the whole set", () => {
    const fresh = [
      entry({ agent_id: 'agent.other', permission: 'allow' }),
      entry({ agent_id: 'agent.other', entry_type: 'tool_grant', tool_name: 'recall', permission: 'deny' }),
      entry({ agent_id: 'agent.a', permission: 'deny' }),
    ];
    const merged = mergeAgentEntries(fresh, 'agent.a', 'cpersona', { server: 'allow', tools: {} }, 'NOW');

    const others = merged.filter((e) => e.agent_id === 'agent.other');
    expect(others).toHaveLength(2);
    expect(others.map((e) => `${e.entry_type}:${e.tool_name ?? ''}:${e.permission}`).sort()).toEqual([
      'server_grant::allow',
      'tool_grant:recall:deny',
    ]);

    const mine = merged.filter((e) => e.agent_id === 'agent.a');
    expect(mine).toEqual([
      {
        entry_type: 'server_grant',
        agent_id: 'agent.a',
        server_id: 'cpersona',
        permission: 'allow',
        granted_by: 'user',
        granted_at: 'NOW',
      },
    ]);
  });

  it('leaves entries on other servers alone', () => {
    const fresh = [entry({ server_id: 'websearch', agent_id: 'agent.a' })];
    const merged = mergeAgentEntries(fresh, 'agent.a', 'cpersona', { server: 'deny', tools: {} }, 'NOW');
    expect(merged.filter((e) => e.server_id === 'websearch')).toHaveLength(1);
  });

  it('writes nothing for this agent when the answer is default, and drops derived capability rows', () => {
    const fresh = [
      entry({ agent_id: 'agent.a', permission: 'allow' }),
      entry({ agent_id: 'agent.a', entry_type: 'capability', permission: 'allow' }),
      entry({ agent_id: 'agent.other' }),
    ];
    const merged = mergeAgentEntries(fresh, 'agent.a', 'cpersona', { server: 'default', tools: {} }, 'NOW');
    expect(merged.filter((e) => e.agent_id === 'agent.a')).toEqual([]);
    expect(merged.some((e) => e.entry_type === 'capability')).toBe(false);
    expect(merged).toHaveLength(1);
  });

  it('writes the per-tool exceptions alongside the server answer', () => {
    const merged = mergeAgentEntries(
      [],
      'agent.a',
      'cscheduler',
      { server: 'allow', tools: { delete_scope: 'deny', create_task: 'allow' } },
      'NOW',
    );
    expect(merged.map((e) => `${e.entry_type}:${e.tool_name ?? '-'}:${e.permission}`).sort()).toEqual([
      'server_grant:-:allow',
      'tool_grant:create_task:allow',
      'tool_grant:delete_scope:deny',
    ]);
  });
});
