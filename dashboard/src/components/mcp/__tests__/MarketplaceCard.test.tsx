import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (k: string) => k }) }));

import type { MarketplaceCatalogEntry } from '../../../types';
import { MarketplaceCard } from '../MarketplaceCard';

function entry(over: Partial<MarketplaceCatalogEntry> = {}): MarketplaceCatalogEntry {
  return {
    id: 'acme-console',
    name: 'Acme console',
    description: 'The operations panel.',
    category: 'tool',
    version: '1.0.0',
    directory: 'acme-console',
    dependencies: [],
    env_vars: [],
    optional_env_vars: [],
    tags: [],
    trust_level: 'standard',
    auto_restart: false,
    icon: null,
    runtime: 'python',
    changelog: null,
    seal: 'sha256:x',
    installed: false,
    installed_version: null,
    update_available: false,
    running: false,
    ...over,
  } as MarketplaceCatalogEntry;
}

describe('MarketplaceCard', () => {
  it('says a restricted entry is published only to you', () => {
    render(<MarketplaceCard entry={entry({ restricted: true })} onInstall={() => {}} onUninstall={() => {}} />);
    expect(screen.getByTestId('restricted-note').textContent).toBe('marketplace.restricted');
  });

  it('says nothing of the kind for a public entry', () => {
    render(<MarketplaceCard entry={entry({ restricted: false })} onInstall={() => {}} onUninstall={() => {}} />);
    expect(screen.queryByTestId('restricted-note')).toBeNull();
  });
});
