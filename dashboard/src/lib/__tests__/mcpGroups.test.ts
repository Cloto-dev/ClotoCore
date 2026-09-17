import { expect, it, describe as suite } from 'vitest';
import type { MarketplaceCatalogEntry, McpServerInfo } from '../../types';
import { describe, deviation, groupServers, matchesQuery, needsAttention, serverKind } from '../mcpGroups';

function server(id: string, over: Partial<McpServerInfo> = {}): McpServerInfo {
  return {
    id,
    command: 'x',
    args: [],
    status: 'Connected',
    tools: [],
    is_cloto_sdk: false,
    ...over,
  } as McpServerInfo;
}

function entry(id: string, over: Partial<MarketplaceCatalogEntry> = {}): MarketplaceCatalogEntry {
  return {
    id,
    name: id,
    description: `${id} from the catalog`,
    category: 'tool',
    version: '1.0.0',
    installed: true,
    update_available: false,
    ...over,
  } as MarketplaceCatalogEntry;
}

suite('the server list', () => {
  it('puts a failing server and one with an update first, whatever their kind', () => {
    const catalog = [entry('cat-mind', { category: 'mind', version: '0.7.2', update_available: true })];
    const groups = groupServers(
      [
        server('ok-tool'),
        server('broken', { status: 'Error', status_message: 'exit 1', tools: ['think'] }),
        server('stale', { marketplace_id: 'cat-mind' }),
      ],
      catalog,
    );
    expect(groups.map((g) => g.group)).toEqual(['attention', 'tool']);
    expect(groups[0].rows.map((r) => r.server.id)).toEqual(['broken', 'stale']);
  });

  it('groups by the catalog kind when there is one, else by the tool surface', () => {
    expect(serverKind(server('a', { tools: ['think'] }))).toBe('mind');
    expect(serverKind(server('b', { tools: ['recall', 'store'] }))).toBe('memory');
    expect(serverKind(server('c', { tools: ['run'] }))).toBe('tool');
    expect(serverKind(server('d'), entry('d', { category: 'voice' }))).toBe('sense');
    expect(serverKind(server('e'), entry('e', { category: 'io' }))).toBe('output');
    // The catalog outranks the surface: an engine the catalog files as a tool is a tool.
    expect(serverKind(server('f', { tools: ['think'] }), entry('f', { category: 'tool' }))).toBe('tool');
  });

  it('orders the bands mind, memory, tool, sense, output and names within a band', () => {
    const catalog = [entry('s', { category: 'vision' }), entry('o', { category: 'output' })];
    const groups = groupServers(
      [
        server('zeta', { tools: ['run'] }),
        server('alpha', { tools: ['run'] }),
        server('out', { marketplace_id: 'o' }),
        server('mem', { tools: ['recall'] }),
        server('see', { marketplace_id: 's' }),
        server('mind', { tools: ['think'] }),
      ],
      catalog,
    );
    expect(groups.map((g) => g.group)).toEqual(['mind', 'memory', 'tool', 'sense', 'output']);
    expect(groups[2].rows.map((r) => r.server.id)).toEqual(['alpha', 'zeta']);
  });
});

suite('what is written before the description', () => {
  it('is nothing for a running server', () => {
    expect(deviation(server('ok'))).toBeNull();
    expect(needsAttention(server('ok'))).toBe(false);
  });

  it('names the failure, the unresolved env, the connection, the stop, or the update', () => {
    expect(deviation(server('a', { status: 'Error', status_message: 'boom' }))).toEqual({
      kind: 'failing',
      message: 'boom',
    });
    expect(deviation(server('b', { status: 'Error', has_unresolved_env: true }))).toEqual({ kind: 'env_unresolved' });
    expect(deviation(server('c', { status: 'Connecting' }))).toEqual({ kind: 'connecting' });
    expect(deviation(server('d', { status: 'Disconnected' }))).toEqual({ kind: 'off' });
    expect(deviation(server('e'), entry('e', { version: '0.7.2', update_available: true }))).toEqual({
      kind: 'update',
      version: '0.7.2',
    });
    // A stopped server is not "needs a hand": stopping is a choice.
    expect(needsAttention(server('d', { status: 'Disconnected' }))).toBe(false);
  });

  it('does not call an uninstalled catalog entry an update', () => {
    expect(deviation(server('e'), entry('e', { installed: false, update_available: true }))).toBeNull();
  });
});

suite('the description', () => {
  it("is the store's, else the catalog's, else nothing", () => {
    expect(describe(server('a', { description: 'Reads disks' }), entry('a'))).toBe('Reads disks');
    expect(describe(server('a', { description: '  ' }), entry('a'))).toBe('a from the catalog');
    expect(describe(server('a'))).toBeNull();
  });

  it('is searched along with the name and the id', () => {
    const row = { server: server('disk_checker', { display_name: 'Disk Checker', description: 'Reads disks' }) };
    expect(matchesQuery(row, 'DISK')).toBe(true);
    expect(matchesQuery(row, 'reads')).toBe(true);
    expect(matchesQuery(row, 'checker')).toBe(true);
    expect(matchesQuery(row, 'memory')).toBe(false);
    expect(matchesQuery(row, '  ')).toBe(true);
  });
});
