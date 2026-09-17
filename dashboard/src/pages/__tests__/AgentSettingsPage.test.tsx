import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata, McpServerInfo } from '../../types';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string, o?: Record<string, unknown>) => (o ? `${k}:${Object.values(o).join('|')}` : k),
    i18n: { language: 'en' },
  }),
}));

const routing = vi.hoisted(() => ({ navigate: vi.fn(), params: { id: 'agent.a' }, search: '' }));
vi.mock('react-router-dom', () => ({
  useNavigate: () => routing.navigate,
  useParams: () => routing.params,
  useSearchParams: () => [new URLSearchParams(routing.search), vi.fn()],
}));

const data = vi.hoisted(() => ({ servers: [] as McpServerInfo[], agents: [] as AgentMetadata[] }));

/**
 * Every mutating call the page could possibly make. The deferred-save test
 * asserts against this whole list, so a call added to the page later cannot
 * slip past Save without being noticed.
 */
const MUTATORS = [
  'updateAgent',
  'setRecallPrecision',
  'uploadAvatar',
  'deleteAvatar',
  'uploadVrm',
  'deleteVrm',
  'putMcpServerAccess',
  'putAgentMcpAccess',
  'setAgentPowerPassword',
  'deleteAgent',
  'toggleAgentPower',
  'createAgent',
] as const;

const apiFns = vi.hoisted(() => {
  const names = [
    'updateAgent',
    'setRecallPrecision',
    'uploadAvatar',
    'deleteAvatar',
    'uploadVrm',
    'deleteVrm',
    'putMcpServerAccess',
    'putAgentMcpAccess',
    'setAgentPowerPassword',
    'deleteAgent',
    'toggleAgentPower',
    'createAgent',
    'getAgentAccess',
    'getAgentInstructionFiles',
    'getMemories',
    'getRecallPrecision',
    'getMcpServerAccess',
    'getMcpServerTools',
    'getAvatarUrl',
  ];
  const obj: Record<string, ReturnType<typeof vi.fn>> = {};
  for (const n of names) obj[n] = vi.fn();
  return obj;
});
vi.mock('../../hooks/useApi', () => ({ useApi: () => apiFns }));
vi.mock('../../hooks/useMcpServers', () => ({
  useMcpServers: () => ({ servers: data.servers, isLoading: false, error: null, refetch: vi.fn() }),
}));
const refetchAgents = vi.hoisted(() => vi.fn());
vi.mock('../../contexts/AgentContext', () => ({
  useAgentContext: () => ({ agents: data.agents, refetchAgents }),
}));
vi.mock('../../components/PowerToggleModal', () => ({ PowerToggleModal: () => <div>power-modal</div> }));
vi.mock('../../components/agents/DeleteAgentModal', () => ({ DeleteAgentModal: () => <div>delete-modal</div> }));
vi.mock('../../components/VrmThumbnailDialog', () => ({ VrmThumbnailDialog: () => null }));

import { AgentSettingsPage } from '../AgentSettingsPage';

function agent(over: Partial<AgentMetadata> = {}): AgentMetadata {
  return {
    id: 'agent.a',
    name: 'Sapphy',
    description: 'Keeps notes.',
    default_engine_id: 'ollama',
    required_capabilities: [],
    enabled: true,
    last_seen: 0,
    status: 'online',
    metadata: { preferred_memory: 'cpersona', agent_type: 'ai' },
    ...over,
  };
}

function server(id: string, tools: string[], status = 'Connected'): McpServerInfo {
  return { id, command: 'x', args: [], status, tools, is_cloto_sdk: false } as McpServerInfo;
}

/** How many times any mutating endpoint was called. */
function mutationCount(): number {
  return MUTATORS.reduce((n, name) => n + apiFns[name].mock.calls.length, 0);
}

beforeEach(() => {
  vi.clearAllMocks();
  // jsdom has no layout, so it has no scrollIntoView. The rail calls it.
  Element.prototype.scrollIntoView = vi.fn();
  routing.params = { id: 'agent.a' };
  routing.search = '';
  data.agents = [agent()];
  data.servers = [server('ollama', ['think']), server('cpersona', ['recall', 'store']), server('cerebras', ['think'])];
  apiFns.getAgentAccess.mockResolvedValue({ entries: [] });
  apiFns.getAgentInstructionFiles.mockResolvedValue({ budget_chars: 9000, files: [] });
  apiFns.getMemories.mockResolvedValue({ memories: [], capabilities: { set_recall_precision: false } });
  apiFns.getMcpServerAccess.mockResolvedValue({ entries: [] });
  apiFns.getMcpServerTools.mockResolvedValue([]);
  apiFns.updateAgent.mockResolvedValue(undefined);
  apiFns.putMcpServerAccess.mockResolvedValue(undefined);
  apiFns.setAgentPowerPassword.mockResolvedValue(undefined);
});

async function draw() {
  const view = render(<AgentSettingsPage />);
  await waitFor(() => expect(apiFns.getAgentAccess).toHaveBeenCalled());
  return view;
}

describe('an engine that runs a CLI harness', () => {
  it('points to the CLI agents page only while such an engine is chosen, without saving anything', async () => {
    data.servers = [server('ollama', ['think']), server('harnesses', ['think', 'probe_harnesses'])];
    await draw();
    expect(screen.queryByTestId('engine-harness-hint')).toBeNull();

    fireEvent.click(screen.getByLabelText('form.llm_engine'));
    fireEvent.pointerDown(screen.getByRole('option', { name: 'harnesses' }));
    const hint = screen.getByTestId('engine-harness-hint');
    fireEvent.click(within(hint).getByRole('button'));
    expect(routing.navigate).toHaveBeenCalledWith('/agents/cli');
    expect(mutationCount()).toBe(0);

    fireEvent.click(screen.getByLabelText('form.llm_engine'));
    fireEvent.pointerDown(screen.getByRole('option', { name: 'ollama' }));
    expect(screen.queryByTestId('engine-harness-hint')).toBeNull();
  });
});

describe('the settings page, before Save', () => {
  it('makes no mutating call while fields are edited', async () => {
    await draw();
    fireEvent.change(screen.getByLabelText('form.name'), { target: { value: 'Renamed' } });
    fireEvent.change(screen.getByLabelText('form.description'), { target: { value: 'Different.' } });
    fireEvent.change(screen.getByLabelText('settings.colour'), { target: { value: 'hsl(20 70% 60%)' } });
    fireEvent.click(screen.getByLabelText('form.llm_engine'));
    fireEvent.pointerDown(screen.getByRole('option', { name: 'cerebras' }));
    fireEvent.click(screen.getByText('recall.timing_session'));
    fireEvent.click(within(screen.getByLabelText('settings.grant_for:cpersona')).getByText('settings.grant_deny'));

    expect(mutationCount()).toBe(0);
  });

  it('makes no mutating call when the changes are discarded, and forgets them', async () => {
    await draw();
    fireEvent.change(screen.getByLabelText('form.name'), { target: { value: 'Renamed' } });
    expect((screen.getByText('settings.discard') as HTMLButtonElement).disabled).toBe(false);

    fireEvent.click(screen.getByText('settings.discard'));
    expect(mutationCount()).toBe(0);
    expect((screen.getByLabelText('form.name') as HTMLInputElement).value).toBe('Sapphy');
    // Nothing is dirty any more, so neither button is live.
    expect((screen.getByText('settings.discard') as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByText('save') as HTMLButtonElement).disabled).toBe(true);
  });

  it('makes no mutating call when the page is left by the back link', async () => {
    await draw();
    fireEvent.change(screen.getByLabelText('form.name'), { target: { value: 'Renamed' } });
    fireEvent.click(screen.getByText(/^‹/));
    expect(mutationCount()).toBe(0);
    expect(routing.navigate).toHaveBeenCalledWith('/');
  });
});

describe('the settings page, on Save', () => {
  it('sends the name, the description, the engine, the routing and the accent — and no password', async () => {
    await draw();
    fireEvent.change(screen.getByLabelText('form.name'), { target: { value: 'Renamed' } });
    fireEvent.change(screen.getByLabelText('form.description'), { target: { value: 'Different.' } });
    fireEvent.click(screen.getByLabelText('form.llm_engine'));
    fireEvent.pointerDown(screen.getByRole('option', { name: 'cerebras' }));
    fireEvent.change(screen.getByLabelText('settings.colour'), { target: { value: 'hsl(20 70% 62%)' } });
    fireEvent.click(screen.getByText('routing.add_rule'));
    fireEvent.change(screen.getByLabelText('routing.match_label:1'), { target: { value: 'tools_likely' } });
    fireEvent.click(screen.getByLabelText('routing.engine_label:1'));
    fireEvent.pointerDown(
      within(screen.getByRole('listbox', { name: 'routing.engine_label:1' })).getByRole('option', { name: 'ollama' }),
    );

    fireEvent.click(screen.getByText('save'));
    await waitFor(() => expect(apiFns.updateAgent).toHaveBeenCalledTimes(1));

    const [id, payload] = apiFns.updateAgent.mock.calls[0];
    expect(id).toBe('agent.a');
    expect(payload.name).toBe('Renamed');
    expect(payload.description).toBe('Different.');
    expect(payload.default_engine_id).toBe('cerebras');
    expect(payload.metadata.accent).toBe('20 70% 62%');
    expect(JSON.parse(payload.metadata.engine_routing)).toEqual([{ match: 'tools_likely', engine: 'ollama' }]);
    expect(payload.metadata.preferred_memory).toBe('cpersona');
    // Untouched: the password endpoint is not called, and nothing avatar-shaped is.
    expect(apiFns.setAgentPowerPassword).not.toHaveBeenCalled();
    expect(apiFns.uploadAvatar).not.toHaveBeenCalled();
    expect(apiFns.deleteAvatar).not.toHaveBeenCalled();
    await waitFor(() => expect(routing.navigate).toHaveBeenCalledWith('/'));
  });

  it('sends the name and leaves the description out when only the name moved', async () => {
    await draw();
    fireEvent.change(screen.getByLabelText('form.name'), { target: { value: 'Renamed' } });
    fireEvent.click(screen.getByText('save'));
    await waitFor(() => expect(apiFns.updateAgent).toHaveBeenCalledTimes(1));
    const payload = apiFns.updateAgent.mock.calls[0][1];
    expect(payload.name).toBe('Renamed');
    expect(payload.description).toBeUndefined();
  });

  it('sends the description and leaves the name out when only the description moved', async () => {
    // The other half of the same rule: a field that did not move is absent, so
    // the kernel is never asked to write a value nobody changed.
    await draw();
    fireEvent.change(screen.getByLabelText('form.description'), { target: { value: 'Different.' } });
    fireEvent.click(screen.getByText('save'));
    await waitFor(() => expect(apiFns.updateAgent).toHaveBeenCalledTimes(1));
    const payload = apiFns.updateAgent.mock.calls[0][1];
    expect(payload.description).toBe('Different.');
    expect(payload.name).toBeUndefined();
  });

  it('sets the password only when it was typed, and carries the current one when one is set', async () => {
    data.agents = [agent({ metadata: { preferred_memory: 'cpersona', has_power_password: 'true' } })];
    await draw();
    fireEvent.change(screen.getByLabelText('settings.password_current'), { target: { value: 'old' } });
    fireEvent.change(screen.getByLabelText('settings.password_new'), { target: { value: 'new' } });
    fireEvent.click(screen.getByText('save'));
    await waitFor(() => expect(apiFns.setAgentPowerPassword).toHaveBeenCalledTimes(1));
    expect(apiFns.setAgentPowerPassword.mock.calls[0]).toEqual(['agent.a', 'new', 'old']);
  });

  it('writes nothing else when the password is refused', async () => {
    data.agents = [agent({ metadata: { has_power_password: 'true' } })];
    apiFns.setAgentPowerPassword.mockRejectedValue(new Error('wrong password'));
    await draw();
    fireEvent.change(screen.getByLabelText('form.name'), { target: { value: 'Renamed' } });
    fireEvent.change(screen.getByLabelText('settings.password_current'), { target: { value: 'guess' } });
    fireEvent.change(screen.getByLabelText('settings.password_new'), { target: { value: 'new' } });
    fireEvent.click(screen.getByText('save'));
    await screen.findByText('wrong password');
    // The refusal is the only mutating call that was made: the rename did not land.
    expect(apiFns.updateAgent).not.toHaveBeenCalled();
    expect(mutationCount()).toBe(1);
  });

  it('removes the password by sending an empty one', async () => {
    data.agents = [agent({ metadata: { has_power_password: 'true' } })];
    await draw();
    fireEvent.click(screen.getByLabelText('settings.password_remove'));
    fireEvent.click(screen.getByText('save'));
    await waitFor(() => expect(apiFns.setAgentPowerPassword).toHaveBeenCalledTimes(1));
    expect(apiFns.setAgentPowerPassword.mock.calls[0][1]).toBe('');
  });

  it('refuses a colour it cannot draw rather than storing it', async () => {
    await draw();
    fireEvent.change(screen.getByLabelText('settings.colour'), { target: { value: 'cornflower' } });
    fireEvent.click(screen.getByText('save'));
    await waitFor(() => expect(screen.getAllByText('settings.accent_invalid').length).toBeGreaterThan(0));
    expect(apiFns.updateAgent).not.toHaveBeenCalled();
  });

  it('grants the engine and the memory server it just chose', async () => {
    await draw();
    fireEvent.click(screen.getByLabelText('form.llm_engine'));
    fireEvent.pointerDown(screen.getByRole('option', { name: 'cerebras' }));
    fireEvent.click(screen.getByText('save'));
    await waitFor(() => expect(apiFns.putMcpServerAccess).toHaveBeenCalled());

    const written = apiFns.putMcpServerAccess.mock.calls.map((c) => c[0]);
    expect(written).toContain('cerebras');
    const entries = apiFns.putMcpServerAccess.mock.calls.find((c) => c[0] === 'cerebras')?.[1];
    expect(entries).toEqual([
      expect.objectContaining({
        entry_type: 'server_grant',
        agent_id: 'agent.a',
        server_id: 'cerebras',
        permission: 'allow',
      }),
    ]);
  });
});

describe('tool permissions', () => {
  it("re-reads the server and keeps the other agents' entries when it writes", async () => {
    const theirs = {
      entry_type: 'server_grant',
      agent_id: 'agent.other',
      server_id: 'cpersona',
      permission: 'allow',
      granted_at: '2026-09-17T00:00:00Z',
    };
    const theirTool = {
      entry_type: 'tool_grant',
      agent_id: 'agent.other',
      server_id: 'cpersona',
      tool_name: 'store',
      permission: 'deny',
      granted_at: '2026-09-17T00:00:00Z',
    };
    // What the agent-scoped read knows, and what the server holds in full.
    apiFns.getAgentAccess.mockResolvedValue({ entries: [] });
    apiFns.getMcpServerAccess.mockResolvedValue({ entries: [theirs, theirTool] });

    await draw();
    fireEvent.click(within(screen.getByLabelText('settings.grant_for:cpersona')).getByText('settings.grant_allow'));
    fireEvent.click(screen.getByText('save'));

    await waitFor(() => expect(apiFns.putMcpServerAccess).toHaveBeenCalled());
    // The fresh read happens before the write, not at page load.
    expect(apiFns.getMcpServerAccess).toHaveBeenCalledWith('cpersona');

    const entries = apiFns.putMcpServerAccess.mock.calls.find((c) => c[0] === 'cpersona')?.[1];
    expect(entries).toEqual(
      expect.arrayContaining([expect.objectContaining(theirs), expect.objectContaining(theirTool)]),
    );
    expect(entries.filter((e: { agent_id: string }) => e.agent_id === 'agent.other')).toHaveLength(2);
    expect(entries.filter((e: { agent_id: string }) => e.agent_id === 'agent.a')).toEqual([
      expect.objectContaining({ entry_type: 'server_grant', permission: 'allow', server_id: 'cpersona' }),
    ]);
  });

  it('does not write a server it could not read', async () => {
    apiFns.getMcpServerAccess.mockRejectedValue(new Error('read failed'));
    await draw();
    fireEvent.click(within(screen.getByLabelText('settings.grant_for:cpersona')).getByText('settings.grant_allow'));
    fireEvent.click(screen.getByText('save'));
    await screen.findByText('read failed');
    // A list built on a failed read holds this agent's rows only; sending it
    // would delete every other agent's grant on the server.
    expect(apiFns.putMcpServerAccess).not.toHaveBeenCalled();
  });

  it('writes only the servers whose answer moved', async () => {
    // Already granted what it uses, so the save has nothing else to add.
    apiFns.getAgentAccess.mockResolvedValue({
      entries: [
        { entry_type: 'server_grant', agent_id: 'agent.a', server_id: 'ollama', permission: 'allow', granted_at: '' },
        { entry_type: 'server_grant', agent_id: 'agent.a', server_id: 'cpersona', permission: 'allow', granted_at: '' },
      ],
    });
    await draw();
    fireEvent.click(within(screen.getByLabelText('settings.grant_for:cpersona')).getByText('settings.grant_deny'));
    fireEvent.click(screen.getByText('save'));
    await waitFor(() => expect(apiFns.putMcpServerAccess).toHaveBeenCalled());
    expect(apiFns.putMcpServerAccess.mock.calls.map((c) => c[0])).toEqual(['cpersona']);
  });

  it('counts a server’s tools, and says how many are denied by name', async () => {
    apiFns.getAgentAccess.mockResolvedValue({
      entries: [
        {
          entry_type: 'tool_grant',
          agent_id: 'agent.a',
          server_id: 'cpersona',
          tool_name: 'store',
          permission: 'deny',
          granted_at: '',
        },
      ],
    });
    await draw();
    await waitFor(() => expect(screen.getByText('settings.tools_with_denied:2|1')).toBeTruthy());
    // The two single-tool engines say so too; the denied count belongs to cpersona alone.
    expect(screen.getAllByText('settings.tools_count:1')).toHaveLength(2);
  });
});

describe('the always-loaded files', () => {
  it('names a file that did not fit, and writes each file as a share of the budget', async () => {
    // A budget that is deliberately not the kernel's current number: a screen
    // that copied the constant instead of reading the answer computes 50% and 67%
    // here, and this says 25% and 33%.
    apiFns.getAgentInstructionFiles.mockResolvedValue({
      budget_chars: 18000,
      files: [
        { name: 'CLAUDE.md', present: true, chars: 4500, loaded: true },
        { name: 'AGENTS.md', present: true, chars: 6000, loaded: false },
        { name: 'MEMORY.md', present: false, chars: 0, loaded: false },
      ],
    });
    await draw();
    await waitFor(() => expect(screen.getByText('settings.left_out:AGENTS.md')).toBeTruthy());
    expect(screen.getByText('settings.file_present:4500|25')).toBeTruthy();
    expect(screen.getByText('settings.file_present:6000|33')).toBeTruthy();
    expect(screen.getByText('settings.file_absent')).toBeTruthy();
  });

  it('says nothing was left out when everything fits', async () => {
    apiFns.getAgentInstructionFiles.mockResolvedValue({
      budget_chars: 400,
      files: [{ name: 'CLAUDE.md', present: true, chars: 100, loaded: true }],
    });
    await draw();
    await waitFor(() => expect(screen.getByText('settings.file_present:100|25')).toBeTruthy());
    expect(screen.queryByText(/^settings\.left_out/)).toBeNull();
  });
});

describe('arriving at a section', () => {
  it('opens on the section the link asked for', async () => {
    routing.search = 'section=tools';
    await draw();
    const rail = screen.getByLabelText('settings.sections');
    const on = within(rail)
      .getAllByRole('button')
      .filter((b) => b.className.includes('on'));
    expect(on.map((b) => b.textContent)).toEqual(['settings.section_tools']);
  });

  it('opens on the basics when the link asks for nothing', async () => {
    await draw();
    const rail = screen.getByLabelText('settings.sections');
    const on = within(rail)
      .getAllByRole('button')
      .filter((b) => b.className.includes('on'));
    expect(on.map((b) => b.textContent)).toEqual(['settings.section_basics']);
  });
});
