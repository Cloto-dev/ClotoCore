import { describe, expect, it } from 'vitest';
import type { MarketplaceCatalogEntry, MarketplaceCollection } from '../types';
import { resolveHubPresets, SERVER_PRESETS } from './presets';

function entry(id: string): MarketplaceCatalogEntry {
  return {
    id,
    name: `Test ${id}`,
    description: 't',
    category: 'tool',
    version: '1.0.0',
    directory: '',
    dependencies: [],
    env_vars: [],
    optional_env_vars: [],
    tags: [],
    trust_level: 'core',
    auto_restart: false,
    runtime: 'python',
    installed: false,
    update_available: false,
    running: false,
  } as MarketplaceCatalogEntry;
}

function collection(id: string, servers: string[], engine?: string): MarketplaceCollection {
  return { id, icon: 'layers', servers, default_engine: engine };
}

describe('resolveHubPresets', () => {
  const catalog = [entry('cpersona'), entry('terminal'), entry('cerebras')];

  it('returns null when the hub served no collections (bundled floor stays)', () => {
    expect(resolveHubPresets(undefined, catalog)).toBeNull();
    expect(resolveHubPresets([], catalog)).toBeNull();
  });

  it('intersects servers with the catalog (ghost ids dropped)', () => {
    const resolved = resolveHubPresets([collection('standard', ['cpersona', 'ghost', 'terminal'])], catalog);
    expect(resolved).not.toBeNull();
    expect(resolved?.[0].servers).toEqual(['cpersona', 'terminal']);
  });

  it('drops a collection whose servers are all ghosts', () => {
    const resolved = resolveHubPresets(
      [collection('all-ghosts', ['g1', 'g2']), collection('ok', ['cpersona'])],
      catalog,
    );
    expect(resolved?.map((p) => p.id)).toEqual(['ok']);
  });

  it('returns null when every collection resolves empty', () => {
    expect(resolveHubPresets([collection('all-ghosts', ['g1'])], catalog)).toBeNull();
  });

  it('keeps a catalog-present default engine and falls back otherwise', () => {
    const resolved = resolveHubPresets(
      [collection('a', ['cpersona'], 'cerebras'), collection('b', ['cpersona'], 'ghost-engine')],
      catalog,
    );
    expect(resolved?.[0].defaultEngine).toBe('cerebras');
    expect(resolved?.[1].defaultEngine).toBe('cerebras'); // fallback
  });

  it('bundled floor presets only reference well-formed shapes', () => {
    for (const p of SERVER_PRESETS) {
      expect(p.servers.length).toBeGreaterThan(0);
      expect(p.defaultEngine).toBeTruthy();
    }
  });
});
