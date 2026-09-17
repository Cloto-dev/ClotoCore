/**
 * One agent's slice of the access-control table, and how to put it back.
 *
 * `PUT /api/mcp/servers/:id/access` **replaces the server's entire entry set**.
 * That makes the dangerous mistake here a quiet one: send a list built only
 * from what this screen knows about this agent, and every other agent's grant
 * on that server is deleted without an error anywhere. So the write path is
 * always read-modify-write against a freshly fetched list, and the modify step
 * is this file — it touches exactly the rows naming this agent and this
 * server, and copies everything else through untouched.
 */

import type { AccessControlEntry, AccessPermission } from '../types';

/** A grant as the UI states it. `default` means "no entry" — the server's own policy decides. */
export type Grant = AccessPermission | 'default';

/** What one agent is allowed on one server. */
export interface ServerGrants {
  server: Grant;
  /** Only the tools that deviate; a tool with no entry inherits the server's answer. */
  tools: Record<string, Grant>;
}

export type AgentGrants = Record<string, ServerGrants>;

export const NO_GRANTS: ServerGrants = { server: 'default', tools: {} };

/** Read one agent's grants out of the entries `GET /mcp/access/by-agent/:id` returns. */
export function grantsFromEntries(entries: AccessControlEntry[], agentId: string): AgentGrants {
  const out: AgentGrants = {};
  const slot = (serverId: string): ServerGrants => {
    if (!out[serverId]) out[serverId] = { server: 'default', tools: {} };
    return out[serverId];
  };
  for (const e of entries) {
    if (e.agent_id !== agentId) continue;
    if (e.entry_type === 'server_grant' && !e.tool_name) {
      slot(e.server_id).server = e.permission;
    } else if (e.entry_type === 'tool_grant' && e.tool_name) {
      slot(e.server_id).tools[e.tool_name] = e.permission;
    }
  }
  return out;
}

export function grantsFor(grants: AgentGrants, serverId: string): ServerGrants {
  return grants[serverId] ?? NO_GRANTS;
}

/** The grants with one server-wide answer replaced. */
export function withServerGrant(grants: AgentGrants, serverId: string, grant: Grant): AgentGrants {
  const current = grantsFor(grants, serverId);
  return { ...grants, [serverId]: { ...current, server: grant } };
}

/** The grants with one tool's answer replaced. `default` removes the entry. */
export function withToolGrant(grants: AgentGrants, serverId: string, tool: string, grant: Grant): AgentGrants {
  const current = grantsFor(grants, serverId);
  const tools = { ...current.tools };
  if (grant === 'default') delete tools[tool];
  else tools[tool] = grant;
  return { ...grants, [serverId]: { ...current, tools } };
}

/** How many of a server's tools this agent is denied by name. */
export function deniedToolCount(grants: AgentGrants, serverId: string): number {
  return Object.values(grantsFor(grants, serverId).tools).filter((g) => g === 'deny').length;
}

function sameServerGrants(a: ServerGrants, b: ServerGrants): boolean {
  if (a.server !== b.server) return false;
  const ka = Object.keys(a.tools).sort();
  const kb = Object.keys(b.tools).sort();
  return ka.length === kb.length && ka.every((k, i) => k === kb[i] && a.tools[k] === b.tools[k]);
}

/** The servers whose entries for this agent differ between two states. */
export function changedServers(before: AgentGrants, after: AgentGrants): string[] {
  const ids = new Set([...Object.keys(before), ...Object.keys(after)]);
  return [...ids].filter((id) => !sameServerGrants(grantsFor(before, id), grantsFor(after, id)));
}

/**
 * The full entry list to PUT for one server: everything the server currently
 * has minus this agent's rows on it, plus this agent's new rows.
 *
 * `capability` entries are dropped, as the server page's save does: they are
 * derived, and echoing them back turns a read into a write of something this
 * screen never edited.
 */
export function mergeAgentEntries(
  fresh: AccessControlEntry[],
  agentId: string,
  serverId: string,
  grants: ServerGrants,
  now: string = new Date().toISOString(),
): AccessControlEntry[] {
  const kept = fresh.filter(
    (e) =>
      e.entry_type !== 'capability' &&
      !(
        e.agent_id === agentId &&
        e.server_id === serverId &&
        (e.entry_type === 'server_grant' || e.entry_type === 'tool_grant')
      ),
  );
  const mine: AccessControlEntry[] = [];
  if (grants.server !== 'default') {
    mine.push({
      entry_type: 'server_grant',
      agent_id: agentId,
      server_id: serverId,
      permission: grants.server,
      granted_by: 'user',
      granted_at: now,
    });
  }
  for (const [tool, grant] of Object.entries(grants.tools)) {
    if (grant === 'default') continue;
    mine.push({
      entry_type: 'tool_grant',
      agent_id: agentId,
      server_id: serverId,
      tool_name: tool,
      permission: grant,
      granted_by: 'user',
      granted_at: now,
    });
  }
  return [...kept, ...mine];
}
