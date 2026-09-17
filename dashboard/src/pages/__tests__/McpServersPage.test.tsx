import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { MarketplaceCatalogEntry, McpServerInfo } from '../../types';

// Echo i18n keys so the assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string, opts?: Record<string, unknown>) =>
      opts && 'count' in opts ? `${k}:${opts.count}${'running' in opts ? `:${opts.running}` : ''}` : k,
    i18n: { language: 'en' },
  }),
}));

const data = vi.hoisted(() => ({
  servers: [] as McpServerInfo[],
  catalog: [] as MarketplaceCatalogEntry[],
}));
// A fixed object: the page reads ?… from it and hands back what it removed.
const router = vi.hoisted(() => ({
  params: new URLSearchParams(),
  setParams: vi.fn(),
}));
vi.mock('react-router-dom', () => ({ useSearchParams: () => [router.params, router.setParams] }));
vi.mock('../../hooks/useApi', () => ({ useApi: () => ({ apiKey: 'k' }) }));
vi.mock('../../hooks/useMcpServers', () => ({
  useMcpServers: () => ({ servers: data.servers, isLoading: false, error: null, refetch: vi.fn() }),
}));
vi.mock('../../hooks/useMarketplace', () => ({
  useMarketplace: () => ({ servers: data.catalog, isLoading: false, error: null, refetch: vi.fn() }),
}));
vi.mock('../../components/mcp/MarketplaceTab', () => ({ MarketplaceTab: () => <div>marketplace</div> }));
vi.mock('../../components/mcp/McpServerDetail', () => ({
  McpServerDetail: ({ server }: { server: McpServerInfo }) => <div>detail:{server.id}</div>,
}));

import { McpServersPage } from '../McpServersPage';

function server(id: string, over: Partial<McpServerInfo> = {}): McpServerInfo {
  return {
    id,
    command: 'x',
    args: [],
    status: 'Connected',
    tools: ['t'],
    is_cloto_sdk: false,
    ...over,
  } as McpServerInfo;
}

beforeEach(() => {
  router.params = new URLSearchParams();
  router.setParams.mockClear();
  data.servers = [
    server('cpersona', { display_name: 'CPersona', tools: ['recall'], description: 'Long-term memory.' }),
    server('disk_checker', { status: 'Error', status_message: 'exit 1', tools: [] }),
    server('deepseek', { tools: ['think'], marketplace_id: 'deepseek' }),
    server('nameless'),
  ];
  data.catalog = [
    {
      id: 'deepseek',
      name: 'DeepSeek',
      description: 'Reasoning over the API.',
      category: 'mind',
      version: '1.2.0',
      installed: true,
      update_available: true,
    } as MarketplaceCatalogEntry,
  ];
});

describe('arriving from search', () => {
  it('opens the server a link names, and takes the name out of the address', () => {
    router.params = new URLSearchParams('server=deepseek&tab=x');
    render(<McpServersPage />);
    expect(screen.getByText('detail:deepseek')).toBeTruthy();
    expect(router.setParams).toHaveBeenCalledTimes(1);
    const [next, opts] = router.setParams.mock.calls[0];
    expect((next as URLSearchParams).get('server')).toBeNull();
    // Only its own parameter is taken.
    expect((next as URLSearchParams).get('tab')).toBe('x');
    expect(opts).toEqual({ replace: true });
  });

  it('opens nothing and changes nothing without one', () => {
    render(<McpServersPage />);
    expect(screen.queryByText(/^detail:/)).toBeNull();
    expect(router.setParams).not.toHaveBeenCalled();
  });
});

describe('the MCP page', () => {
  it('counts the servers, the running ones and the failing ones in the head', () => {
    render(<McpServersPage />);
    expect(screen.getByText(/list\.summary:4:3/)).toBeTruthy();
    expect(screen.getByText('list.failing:1')).toBeTruthy();
  });

  it('draws the ones needing a hand first, then each kind, with a role beside every name it has', () => {
    render(<McpServersPage />);
    const bands = screen.getAllByText(/^groups\./).map((el) => el.textContent);
    expect(bands).toEqual(['groups.attention', 'groups.memory', 'groups.tool']);
    const rows = screen.getAllByRole('button', { name: /cpersona|disk_checker|deepseek|nameless/i });
    expect(rows.map((r) => r.querySelector('small')?.textContent)).toEqual([
      'deepseek',
      'disk_checker',
      'cpersona',
      'nameless',
    ]);
    // The failing one says so; the one with an update names the version; a
    // running one says nothing before its description.
    expect(screen.getByText('deviation.failing_with')).toBeTruthy();
    expect(screen.getByText('deviation.update')).toBeTruthy();
    expect(screen.getByText('Long-term memory.')).toBeTruthy();
    // The catalog's description stands in when the store has none.
    expect(screen.getByText('Reasoning over the API.')).toBeTruthy();
    expect(screen.queryByText('deviation.off')).toBeNull();
  });

  it('searches names, ids and descriptions', () => {
    render(<McpServersPage />);
    fireEvent.change(screen.getByLabelText('list.search'), { target: { value: 'memory' } });
    expect(screen.getByRole('button', { name: 'CPersona' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'disk_checker' })).toBeNull();
  });

  it('counts the updates on their tab and lists them there', () => {
    render(<McpServersPage />);
    fireEvent.click(screen.getByRole('tab', { name: /list.tab_updates/ }));
    expect(screen.getByRole('button', { name: 'deepseek' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'CPersona' })).toBeNull();
  });

  it('opens a server in place of the list', () => {
    render(<McpServersPage />);
    fireEvent.click(screen.getByRole('button', { name: 'CPersona' }));
    expect(screen.getByText('detail:cpersona')).toBeTruthy();
    expect(screen.queryByText('groups.memory')).toBeNull();
  });
});
