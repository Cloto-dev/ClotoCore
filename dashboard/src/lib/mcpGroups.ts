import type { MarketplaceCatalogEntry, McpServerInfo, ServerCategory } from '../types';
import { isEngineServer, isMemoryServer } from './serverCategory';

/**
 * How the workshop's server list is grouped and worded
 * (docs/gui/samples/04-mcp-servers.html). Groups are the capability kinds of
 * docs/PROJECT_VISION.md §5; the servers that need a hand come first, whatever
 * their kind. A state is written only when it deviates — a running server says
 * nothing.
 */

export type ServerGroup = 'attention' | 'mind' | 'memory' | 'tool' | 'sense' | 'output';

/** The order the bands are drawn in. */
export const GROUP_ORDER: ServerGroup[] = ['attention', 'mind', 'memory', 'tool', 'sense', 'output'];

export type Deviation =
  | { kind: 'failing'; message: string | null }
  | { kind: 'env_unresolved' }
  | { kind: 'connecting' }
  | { kind: 'off' }
  | { kind: 'update'; version: string }
  | null;

/** The catalog's kind for a server, by its catalog id; else by its tool surface. */
export function serverKind(server: McpServerInfo, entry?: MarketplaceCatalogEntry): Exclude<ServerGroup, 'attention'> {
  if (entry) return kindOfCategory(entry.category);
  if (isEngineServer(server)) return 'mind';
  if (isMemoryServer(server)) return 'memory';
  return 'tool';
}

function kindOfCategory(category: ServerCategory): Exclude<ServerGroup, 'attention'> {
  switch (category) {
    case 'mind':
      return 'mind';
    case 'memory':
      return 'memory';
    case 'vision':
    case 'voice':
      return 'sense';
    case 'output':
    case 'io':
      return 'output';
    default:
      return 'tool';
  }
}

/** What, if anything, is written before the server's description. */
export function deviation(server: McpServerInfo, entry?: MarketplaceCatalogEntry): Deviation {
  if (server.status === 'Error') {
    return server.has_unresolved_env
      ? { kind: 'env_unresolved' }
      : { kind: 'failing', message: server.status_message ?? null };
  }
  if (server.status === 'Connecting' || server.status === 'Restarting' || server.status === 'Registered') {
    return { kind: 'connecting' };
  }
  if (server.status === 'Disconnected') return { kind: 'off' };
  if (entry?.installed && entry.update_available) return { kind: 'update', version: entry.version };
  return null;
}

/** A server needs a hand when it is failing or has an update waiting. */
export function needsAttention(server: McpServerInfo, entry?: MarketplaceCatalogEntry): boolean {
  const d = deviation(server, entry);
  return d !== null && (d.kind === 'failing' || d.kind === 'env_unresolved' || d.kind === 'update');
}

/** The one line that says what the server is for: the store's, else the
 * catalog's, else nothing — never a line made up here. */
export function describe(server: McpServerInfo, entry?: MarketplaceCatalogEntry): string | null {
  const own = server.description?.trim();
  if (own) return own;
  const fromCatalog = entry?.description?.trim();
  return fromCatalog || null;
}

/** The catalog entry for an installed server, by the id the install recorded. */
export function catalogEntryFor(
  server: McpServerInfo,
  catalog: MarketplaceCatalogEntry[],
): MarketplaceCatalogEntry | undefined {
  if (!server.marketplace_id) return undefined;
  return catalog.find((e) => e.id === server.marketplace_id);
}

export interface ServerRow {
  server: McpServerInfo;
  entry?: MarketplaceCatalogEntry;
}

export interface ServerGroupRows {
  group: ServerGroup;
  rows: ServerRow[];
}

/** The list as drawn: those needing a hand first, then each kind in order;
 * within a group, by name. Empty groups are not drawn. */
export function groupServers(servers: McpServerInfo[], catalog: MarketplaceCatalogEntry[]): ServerGroupRows[] {
  const buckets = new Map<ServerGroup, ServerRow[]>();
  for (const server of servers) {
    const entry = catalogEntryFor(server, catalog);
    const group: ServerGroup = needsAttention(server, entry) ? 'attention' : serverKind(server, entry);
    const rows = buckets.get(group) ?? [];
    rows.push({ server, entry });
    buckets.set(group, rows);
  }
  return GROUP_ORDER.filter((g) => buckets.has(g)).map((group) => ({
    group,
    rows: (buckets.get(group) ?? []).sort((a, b) => nameOf(a.server).localeCompare(nameOf(b.server))),
  }));
}

export function nameOf(server: McpServerInfo): string {
  return server.display_name || server.id;
}

/** Search covers the name, the id and the description. */
export function matchesQuery(row: ServerRow, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const hay = [row.server.id, row.server.display_name ?? '', describe(row.server, row.entry) ?? '']
    .join('\n')
    .toLowerCase();
  return hay.includes(q);
}
