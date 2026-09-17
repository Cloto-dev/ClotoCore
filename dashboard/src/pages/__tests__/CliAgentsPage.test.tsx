import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata, McpServerInfo } from '../../types';

const i18n = vi.hoisted(() => ({
  t: (k: string, o?: Record<string, unknown>) => (o ? `${k}:${Object.values(o).join('|')}` : k),
}));
vi.mock('react-i18next', () => ({ useTranslation: () => i18n }));
const navigate = vi.hoisted(() => vi.fn());
vi.mock('react-router-dom', () => ({ useNavigate: () => navigate }));

// Fixed objects: a hook that hands back a new array every render loops any
// effect that depends on it.
const data = vi.hoisted(() => ({ servers: [] as McpServerInfo[] }));
vi.mock('../../hooks/useMcpServers', () => ({ useMcpServers: () => data }));
const ctx = vi.hoisted(() => ({ agents: [] as AgentMetadata[], refetchAgents: vi.fn() }));
vi.mock('../../contexts/AgentContext', () => ({ useAgentContext: () => ctx }));
const api = vi.hoisted(() => ({
  callMcpTool: vi.fn(),
  getMarketplaceCatalog: vi.fn(),
  getMcpServerSettings: vi.fn(),
  updateMcpServerSettings: vi.fn(),
  getAgents: vi.fn(),
  updateAgent: vi.fn(),
}));
vi.mock('../../hooks/useApi', () => ({ useApi: () => api }));

import { CliAgentsPage } from '../CliAgentsPage';

const connector = (status = 'Connected') =>
  ({ id: 'tools.cli', status, tools: ['think', 'probe_harnesses'] }) as unknown as McpServerInfo;

const SCHEMA = {
  metadata_key: 'cli_agent',
  fields: [
    { key: 'harness', label: 'Harness', input: 'select', options: ['claude', 'codex'] },
    { key: 'model', label: 'Model', input: 'text' },
  ],
};

function probe(harnesses: unknown[], active: string | null = null) {
  return {
    content: [
      {
        type: 'text',
        text: JSON.stringify({
          harnesses,
          configured_harness: null,
          active_harness: active,
          working_directory: '/srv/work',
          agent_config: SCHEMA,
        }),
      },
    ],
  };
}

const CLAUDE = { id: 'claude', label: 'Claude Code', installed: true, version: '2.1.263', plan: 'subscription' };
const CODEX = { id: 'codex', installed: true, version: '0.153.4', plan: 'api_key', credential_store: 'file' };
const GONE = { id: 'gemini', installed: false, binary: 'gemini' };

const agent = (id: string, engine: string, binding?: string): AgentMetadata =>
  ({
    id,
    name: id,
    default_engine_id: engine,
    enabled: true,
    metadata: binding === undefined ? { keep: 'me' } : { keep: 'me', cli_agent: binding },
  }) as unknown as AgentMetadata;

beforeEach(() => {
  vi.clearAllMocks();
  data.servers = [connector()];
  ctx.agents = [];
  api.callMcpTool.mockResolvedValue(probe([CLAUDE, CODEX], 'claude'));
  api.getMarketplaceCatalog.mockResolvedValue({
    servers: [
      { id: 'tools.other', optional_env_vars: [{ name: 'NOT_OURS' }] },
      {
        id: 'tools.cli',
        optional_env_vars: [
          { name: 'OPT_ONE', description: 'first', default: '300' },
          { name: 'OPT_TWO', description: 'second' },
        ],
      },
    ],
  });
  api.getMcpServerSettings.mockResolvedValue({ env: { OPT_ONE: '900', SECRET: '***' } });
  api.updateMcpServerSettings.mockResolvedValue(undefined);
  api.updateAgent.mockResolvedValue(undefined);
});

const pane = () => document.querySelector('.pane') as HTMLElement;
const rows = () => Array.from(document.querySelectorAll('button.harness')) as HTMLButtonElement[];

describe('the three states of the harness list', () => {
  it('says there is no connector, and scans nothing, when none can run a harness', async () => {
    data.servers = [{ id: 'x', status: 'Connected', tools: ['think'] } as unknown as McpServerInfo];
    render(<CliAgentsPage />);
    expect(screen.getByTestId('cli-no-connector').textContent).toContain('cli_agent.not_installed');
    fireEvent.click(screen.getByText('cli_agent.open_mcp'));
    expect(navigate).toHaveBeenCalledWith('/mcp-servers');
    expect(api.callMcpTool).not.toHaveBeenCalled();
  });

  it('draws no harness when the probe found none, and says so', async () => {
    api.callMcpTool.mockResolvedValue(probe([], null));
    render(<CliAgentsPage />);
    await screen.findByText('cli_agent.none_found');
    expect(rows()).toHaveLength(0);
  });

  it('says none was found when the connector lists harnesses but none is installed', async () => {
    api.callMcpTool.mockResolvedValue(probe([GONE, { id: 'codex', installed: false, binary: 'codex' }], null));
    render(<CliAgentsPage />);
    await screen.findByText('cli_agent.none_found');
    // Each is still drawn, saying it was not found.
    expect(rows()).toHaveLength(2);
    expect(rows()[0].textContent).toContain('cli_agent.not_found');
  });

  it('does not say none was found when one of several is installed', async () => {
    api.callMcpTool.mockResolvedValue(probe([GONE, CLAUDE], 'claude'));
    render(<CliAgentsPage />);
    await waitFor(() => expect(rows()).toHaveLength(2));
    expect(screen.queryByText('cli_agent.none_found')).toBeNull();
  });

  it('draws one harness, selected, with its state', async () => {
    api.callMcpTool.mockResolvedValue(probe([CLAUDE], 'claude'));
    render(<CliAgentsPage />);
    await waitFor(() => expect(rows()).toHaveLength(1));
    expect(rows()[0].getAttribute('aria-pressed')).toBe('true');
    expect(within(pane()).getByRole('heading', { level: 2, name: 'Claude Code' })).toBeTruthy();
    expect(screen.getByTestId('cli-state').textContent).toContain('2.1.263');
  });

  it('selects what the connector would run, not the first one listed, when there are several', async () => {
    api.callMcpTool.mockResolvedValue(probe([CLAUDE, CODEX, GONE], 'codex'));
    render(<CliAgentsPage />);
    await waitFor(() => expect(rows()).toHaveLength(3));
    expect(rows().map((r) => r.getAttribute('aria-pressed'))).toEqual(['false', 'true', 'false']);
    expect(within(pane()).getAllByRole('heading', { level: 2 })[0].textContent).toBe('codex');
    fireEvent.click(rows()[2]);
    expect(within(pane()).getAllByRole('heading', { level: 2 })[0].textContent).toBe('gemini');
    expect(screen.getByTestId('cli-state').textContent).toContain('gemini');
    expect(screen.getByTestId('cli-state').textContent).toContain('cli_agent.state_missing');
  });

  it('warns about per-token billing only on a harness set up with an API key', async () => {
    render(<CliAgentsPage />);
    await waitFor(() => expect(rows()).toHaveLength(2));
    expect(screen.queryByText('cli_agent.metered_warning')).toBeNull();
    fireEvent.click(rows()[1]);
    expect(screen.getByText('cli_agent.metered_warning')).toBeTruthy();
  });

  it('does not scan a connector that is not running, and still shows its options', async () => {
    data.servers = [connector('Disconnected')];
    render(<CliAgentsPage />);
    await screen.findByLabelText('OPT_ONE');
    expect(api.callMcpTool).not.toHaveBeenCalled();
    expect(screen.getByText('cli_agent.offline_hint')).toBeTruthy();
  });
});

describe('connection options', () => {
  it("are the connector's own catalog entry, with the stored values, and nobody else's", async () => {
    render(<CliAgentsPage />);
    const one = (await screen.findByLabelText('OPT_ONE')) as HTMLInputElement;
    expect(one.value).toBe('900');
    expect((screen.getByLabelText('OPT_TWO') as HTMLInputElement).placeholder).toBe('cli_agent.option_unset');
    expect(screen.queryByLabelText('NOT_OURS')).toBeNull();
  });

  it('says it could not read them instead of showing an empty form', async () => {
    api.getMcpServerSettings.mockRejectedValue(new Error('down'));
    render(<CliAgentsPage />);
    await screen.findByText('down');
    expect(screen.queryByLabelText('OPT_ONE')).toBeNull();
    expect(screen.queryByText('cli_agent.no_options')).toBeNull();
  });

  it('names every key the server holds at save time, so none is dropped, then scans again', async () => {
    render(<CliAgentsPage />);
    fireEvent.change(await screen.findByLabelText('OPT_TWO'), { target: { value: '40' } });
    // A key set elsewhere after the page read the settings.
    api.getMcpServerSettings.mockResolvedValue({ env: { OPT_ONE: '900', SECRET: '***', LATE: 'x' } });
    const scansBefore = api.callMcpTool.mock.calls.length;
    await act(async () => {
      fireEvent.click(screen.getByText('save'));
    });
    await waitFor(() => expect(api.updateMcpServerSettings).toHaveBeenCalledTimes(1));
    expect(api.updateMcpServerSettings).toHaveBeenCalledWith('tools.cli', {
      env: { OPT_ONE: '***', SECRET: '***', LATE: '***', OPT_TWO: '40' },
    });
    await waitFor(() => expect(api.callMcpTool.mock.calls.length).toBe(scansBefore + 1));
    expect(api.updateAgent).not.toHaveBeenCalled();
  });
});

describe('per-agent settings', () => {
  beforeEach(() => {
    ctx.agents = [
      agent('on-codex', 'tools.cli', '{"harness":"codex","model":"o5"}'),
      agent('on-default', 'tools.cli'),
      agent('broken', 'tools.cli', 'not-json'),
      agent('elsewhere', 'deepseek'),
    ];
  });

  it('lists each agent under the harness its next run uses, and a broken one under every harness', async () => {
    render(<CliAgentsPage />);
    await waitFor(() => expect(rows()).toHaveLength(2));
    // claude is what the connector would pick, so the agent with no choice is here.
    expect(screen.getByTestId('binding-on-default')).toBeTruthy();
    expect(screen.queryByTestId('binding-on-codex')).toBeNull();
    expect(screen.getByTestId('binding-broken').textContent).toContain('cli_agent.binding_invalid');
    expect(screen.queryByTestId('binding-elsewhere')).toBeNull();
    // The broken one cannot be edited safely, so it offers no editor.
    expect(within(screen.getByTestId('binding-broken')).queryByRole('button')).toBeNull();

    fireEvent.click(rows()[1]);
    expect(screen.getByTestId('binding-on-codex').textContent).toContain('Model o5');
    expect(screen.queryByTestId('binding-on-default')).toBeNull();
    expect(screen.getByTestId('binding-broken')).toBeTruthy();
    // The count on each harness is who runs on it.
    expect(rows()[0].textContent).toContain('cli_agent.used_by:1');
    expect(rows()[1].textContent).toContain('cli_agent.used_by:1');
  });

  it('saves an agent from the row as it is at save time, keeping every other metadata key', async () => {
    render(<CliAgentsPage />);
    await waitFor(() => expect(rows()).toHaveLength(2));
    fireEvent.click(rows()[1]);
    fireEvent.click(
      within(screen.getByTestId('binding-on-codex')).getByRole('button', { name: 'cli_agent.edit_binding' }),
    );
    fireEvent.change(screen.getByLabelText('on-codex Model'), { target: { value: 'o6' } });
    expect(screen.getByTestId('binding-on-codex').textContent).toContain('cli_agent.unsaved');

    api.getAgents.mockResolvedValue([
      { ...ctx.agents[0], metadata: { keep: 'me', added: 'meanwhile', cli_agent: '{"harness":"codex","model":"o5"}' } },
    ]);
    await act(async () => {
      fireEvent.click(screen.getByText('save'));
    });
    await waitFor(() => expect(api.updateAgent).toHaveBeenCalledTimes(1));
    const [id, payload] = api.updateAgent.mock.calls[0];
    expect(id).toBe('on-codex');
    expect(payload.metadata.keep).toBe('me');
    expect(payload.metadata.added).toBe('meanwhile');
    expect(JSON.parse(payload.metadata.cli_agent)).toEqual({ harness: 'codex', model: 'o6' });
    expect(api.updateMcpServerSettings).not.toHaveBeenCalled();
    expect(ctx.refetchAgents).toHaveBeenCalled();
  });

  it('writes nothing at all when an edited agent is gone by save time, not even the options', async () => {
    render(<CliAgentsPage />);
    await waitFor(() => expect(rows()).toHaveLength(2));
    fireEvent.click(
      within(screen.getByTestId('binding-on-default')).getByRole('button', { name: 'cli_agent.edit_binding' }),
    );
    fireEvent.change(screen.getByLabelText('on-default Model'), { target: { value: 'haiku' } });
    fireEvent.change(screen.getByLabelText('OPT_TWO'), { target: { value: '40' } });
    api.getAgents.mockResolvedValue([]);
    await act(async () => {
      fireEvent.click(screen.getByText('save'));
    });
    await screen.findByText('cli_agent.agent_missing');
    expect(api.updateAgent).not.toHaveBeenCalled();
    expect(api.updateMcpServerSettings).not.toHaveBeenCalled();
  });

  it('writes the agents before the options', async () => {
    render(<CliAgentsPage />);
    await waitFor(() => expect(rows()).toHaveLength(2));
    fireEvent.click(
      within(screen.getByTestId('binding-on-default')).getByRole('button', { name: 'cli_agent.edit_binding' }),
    );
    fireEvent.change(screen.getByLabelText('on-default Model'), { target: { value: 'haiku' } });
    fireEvent.change(screen.getByLabelText('OPT_TWO'), { target: { value: '40' } });
    api.getAgents.mockResolvedValue([ctx.agents[1]]);
    await act(async () => {
      fireEvent.click(screen.getByText('save'));
    });
    await waitFor(() => expect(api.updateMcpServerSettings).toHaveBeenCalledTimes(1));
    expect(api.updateAgent.mock.invocationCallOrder[0]).toBeLessThan(
      api.updateMcpServerSettings.mock.invocationCallOrder[0],
    );
  });

  it('discards without calling anything', async () => {
    render(<CliAgentsPage />);
    const two = (await screen.findByLabelText('OPT_TWO')) as HTMLInputElement;
    fireEvent.change(two, { target: { value: '40' } });
    fireEvent.click(screen.getByText('settings.discard'));
    expect(two.value).toBe('');
    expect(api.updateMcpServerSettings).not.toHaveBeenCalled();
    expect(api.updateAgent).not.toHaveBeenCalled();
    expect((screen.getByText('save') as HTMLButtonElement).disabled).toBe(true);
  });
});
