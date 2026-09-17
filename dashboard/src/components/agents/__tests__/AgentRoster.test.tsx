import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata, Conversation, McpServerInfo } from '../../../types';

// Echo i18n keys and their values so the assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string, o?: Record<string, unknown>) => (o ? `${k}:${Object.values(o).join('|')}` : k),
    i18n: { language: 'en' },
  }),
}));

const navigate = vi.hoisted(() => vi.fn());
vi.mock('react-router-dom', () => ({ useNavigate: () => navigate }));

// Fixed objects: a hook that hands back a new array every render loops any
// effect that depends on it (runbook §3).
const data = vi.hoisted(() => ({
  servers: [] as McpServerInfo[],
  conversations: [] as Conversation[],
}));
const apiFns = vi.hoisted(() => ({
  getMemories: vi.fn(),
  getAgentAccess: vi.fn(),
  listCronJobs: vi.fn(),
  getNotifications: vi.fn(),
  markNotificationRead: vi.fn(),
  getAvatarUrl: vi.fn(() => 'avatar.png'),
  createAgent: vi.fn(),
  getAgents: vi.fn(),
  putAgentMcpAccess: vi.fn(),
  apiKey: 'k',
}));
vi.mock('../../../hooks/useApi', () => ({ useApi: () => apiFns }));
vi.mock('../../../hooks/useMcpServers', () => ({
  useMcpServers: () => ({ servers: data.servers, isLoading: false, error: null, refetch: vi.fn() }),
}));
vi.mock('../../../contexts/ConversationContext', () => ({
  useConversations: () => ({ conversations: data.conversations }),
}));
vi.mock('../../../hooks/useEventStream', () => ({ useEventStream: () => {} }));
vi.mock('../../PowerToggleModal', () => ({ PowerToggleModal: () => <div>power-modal</div> }));
vi.mock('../CreateAgentModal', () => ({
  CreateAgentModal: ({
    onCreated,
  }: {
    onCreated: (c: { name: string; id: string | null; faceProblem: string | null }) => void;
  }) => (
    <div>
      create-modal
      <button type="button" onClick={() => onCreated({ name: 'Nova', id: 'agent.nova', faceProblem: null })}>
        made-whole
      </button>
      <button type="button" onClick={() => onCreated({ name: 'Nova', id: 'agent.nova', faceProblem: 'disk full' })}>
        made-faceless
      </button>
    </div>
  ),
}));
vi.mock('../DeleteAgentModal', () => ({ DeleteAgentModal: () => <div>delete-modal</div> }));

import { AgentRoster } from '../AgentRoster';

function agent(id: string, over: Partial<AgentMetadata> = {}): AgentMetadata {
  return {
    id,
    name: id.replace('agent.', ''),
    description: 'A role.',
    required_capabilities: [],
    enabled: true,
    last_seen: 0,
    status: 'online',
    metadata: {},
    ...over,
  };
}

function conversation(agentId: string, updatedAt: number): Conversation {
  return {
    id: `c-${agentId}`,
    agent_id: agentId,
    user_id: 'u',
    title: 't',
    created_at: updatedAt,
    updated_at: updatedAt,
    archived_at: null,
    message_count: 1,
  };
}

function notification(over: Record<string, unknown> = {}) {
  return {
    item_id: 'n1',
    kind: 'approval',
    severity: 'warning',
    agent_id: 'agent.busy',
    title: 'May I?',
    body: null,
    created_at: '2026-09-17T00:00:00Z',
    read_at: null,
    resolved_at: null,
    decision: null,
    blocking: true,
    metadata: null,
    ...over,
  };
}

const AGENTS = [agent('agent.busy'), agent('agent.quiet'), agent('agent.off', { enabled: false })];

beforeEach(() => {
  vi.clearAllMocks();
  sessionStorage.clear();
  data.servers = [];
  data.conversations = [];
  apiFns.getMemories.mockResolvedValue({ memories: [], capabilities: {} });
  apiFns.getAgentAccess.mockResolvedValue({ entries: [] });
  apiFns.listCronJobs.mockResolvedValue({ jobs: [] });
  apiFns.getNotifications.mockResolvedValue([]);
  apiFns.markNotificationRead.mockResolvedValue(undefined);
});

function draw(processing: string[] = [], agents: AgentMetadata[] = AGENTS) {
  return render(
    <AgentRoster agents={agents} onSelectAgent={vi.fn()} onRefresh={vi.fn()} processing={new Set(processing)} />,
  );
}

/** The rows, in the order they are drawn. */
function rowNames(): string[] {
  return screen
    .getAllByRole('button')
    .filter((b) => b.className.split(' ').includes('r'))
    .map((b) => b.querySelector('.nm')?.textContent ?? '');
}

describe('the roster', () => {
  it('puts the agents that are answering first, and the rest under idle', () => {
    draw(['agent.busy']);
    const groups = screen.getAllByText(/^roster\.group_/).map((el) => el.textContent);
    expect(groups).toEqual(['roster.group_running', 'roster.group_idle']);
    expect(rowNames()).toEqual(['busy', 'quiet', 'off']);
  });

  it('draws no "running now" band when nobody is answering', () => {
    draw([]);
    expect(screen.queryByText('roster.group_running')).toBeNull();
    expect(screen.getByText('roster.group_idle')).toBeTruthy();
  });

  it('says what each agent is doing: responding, idle, or stopped', () => {
    draw(['agent.busy']);
    expect(screen.getByText('roster.state_responding')).toBeTruthy();
    expect(screen.getByText('roster.state_idle')).toBeTruthy();
    expect(screen.getByText('roster.state_stopped')).toBeTruthy();
    // Only the running row carries the live mark, and only the disabled one is dimmed.
    expect(screen.getByText('roster.state_responding').className).toContain('live');
    expect(screen.getByText('roster.state_idle').className).not.toContain('live');
    const off = screen.getAllByRole('button').find((b) => b.querySelector('.nm')?.textContent === 'off');
    expect(off?.className).toContain('off');
  });

  it('adds the latest thinking step to the state line when the console left one', () => {
    sessionStorage.setItem(
      'cloto-thinking-agent.busy',
      JSON.stringify([{ id: 1, status: 'thought', text: 'reading the file', ts: 0 }]),
    );
    draw(['agent.busy']);
    expect(screen.getByText('roster.state_responding_step:reading the file')).toBeTruthy();
    expect(screen.queryByText('roster.state_responding')).toBeNull();
  });

  it('counts the agents and the running ones in the head', () => {
    draw(['agent.busy']);
    expect(screen.getByText('roster.summary:3|1')).toBeTruthy();
  });

  it('writes the time of the newest conversation on the row, and nothing when there is none', () => {
    data.conversations = [conversation('agent.busy', Date.now() - 60_000), conversation('agent.busy', Date.now())];
    draw([]);
    const busy = screen.getAllByRole('button').find((b) => b.querySelector('.nm')?.textContent === 'busy');
    expect(busy?.querySelector('.last')?.textContent).toMatch(/^\d{2}:\d{2}$/);
    const quiet = screen.getAllByRole('button').find((b) => b.querySelector('.nm')?.textContent === 'quiet');
    expect(quiet?.querySelector('.last')?.textContent).toBe('');
  });

  it('says on the roster when a new agent was made but their face could not be saved', () => {
    const onRefresh = vi.fn();
    render(<AgentRoster agents={AGENTS} onSelectAgent={vi.fn()} onRefresh={onRefresh} processing={new Set()} />);
    fireEvent.click(screen.getByText('create_agent'));
    fireEvent.click(screen.getByText('made-faceless'));
    expect(screen.getByRole('alert').textContent).toBe('create.face_not_saved:Nova|disk full');
    // The agent exists: the dialog is gone and the list is read again.
    expect(screen.queryByText('create-modal')).toBeNull();
    expect(onRefresh).toHaveBeenCalledTimes(1);

    // The next creation that goes well takes the line away.
    fireEvent.click(screen.getByText('create_agent'));
    fireEvent.click(screen.getByText('made-whole'));
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('opens the CLI agents page, not a dialog, from the head', () => {
    draw();
    fireEvent.click(screen.getByText('cli_agent.open'));
    expect(navigate).toHaveBeenCalledWith('/agents/cli');
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('shows the empty state when there is nobody yet', () => {
    draw([], []);
    expect(screen.getByText('no_agents')).toBeTruthy();
  });
});

describe('the unread mark', () => {
  it('marks an agent with an unanswered question, and not one that only raised a notice', async () => {
    apiFns.getNotifications.mockResolvedValue([
      notification({ item_id: 'q', agent_id: 'agent.busy', kind: 'approval' }),
      notification({ item_id: 'n', agent_id: 'agent.quiet', kind: 'notice' }),
    ]);
    draw([]);
    await waitFor(() => expect(screen.getAllByLabelText('roster.unread')).toHaveLength(1));
    const marked = screen.getAllByRole('button').find((b) => b.querySelector('.unread'));
    expect(marked?.querySelector('.nm')?.textContent).toContain('busy');
  });
});

describe('the detail', () => {
  it('reads the agent out in a sentence per line, and names the routing target', async () => {
    data.servers = [
      { id: 'cpersona', command: 'x', args: [], status: 'Connected', tools: ['recall', 'store'], is_cloto_sdk: false },
      { id: 'ollama', command: 'x', args: [], status: 'Connected', tools: ['think'], is_cloto_sdk: false },
    ] as McpServerInfo[];
    apiFns.getMemories.mockResolvedValue({ memories: [{}, {}, {}], capabilities: {} });
    apiFns.getAgentAccess.mockResolvedValue({
      entries: [
        {
          entry_type: 'server_grant',
          agent_id: 'agent.busy',
          server_id: 'cpersona',
          permission: 'allow',
          granted_at: '',
        },
      ],
    });
    apiFns.listCronJobs.mockResolvedValue({
      jobs: [
        { id: 'j1', name: 'morning summary', next_run_at: new Date(2026, 0, 1, 8, 0).getTime() },
        { id: 'j2', name: 'review', next_run_at: new Date(2026, 0, 1, 22, 0).getTime() },
        { id: 'j3', name: 'third', next_run_at: 0 },
      ],
    });

    draw(
      [],
      [
        agent('agent.busy', {
          default_engine_id: 'ollama',
          metadata: {
            preferred_memory: 'cpersona',
            engine_routing: JSON.stringify([{ match: 'tools_likely', engine: 'cerebras' }]),
          },
        }),
      ],
    );

    const row = screen.getAllByRole('button').find((b) => b.className.split(' ').includes('r'));
    if (!row) throw new Error('no roster row');
    fireEvent.click(row);

    expect(screen.getByText('roster.engine_with_routing:ollama|cerebras')).toBeTruthy();
    await waitFor(() => expect(screen.getByText('roster.memory_with_count:cpersona|3')).toBeTruthy());
    // Two tools on the one granted server, one server granted.
    expect(screen.getByText('roster.tools_summary:1|2')).toBeTruthy();
    // The separator is the mocked key itself; what matters is that two jobs are
    // named with their times and the third is only counted.
    expect(screen.getByText('roster.cron_summary:3|08:00 morning summaryroster.cron_sep22:00 review')).toBeTruthy();
  });

  it('opens the settings route, and the tool section when that is what was asked for', () => {
    draw([]);
    const row = screen.getAllByRole('button').find((b) => b.className.split(' ').includes('r'));
    if (!row) throw new Error('no roster row');
    fireEvent.click(row);

    fireEvent.click(screen.getByText('roster.open_settings'));
    expect(navigate).toHaveBeenCalledWith('/agents/agent.busy/settings');

    fireEvent.click(screen.getByText('roster.tool_permissions'));
    expect(navigate).toHaveBeenCalledWith('/agents/agent.busy/settings?section=tools');
  });

  it('offers no export and no delete for the agent that ships with the product', () => {
    draw([], [agent('agent.cloto_default'), agent('agent.mine')]);
    const rows = screen.getAllByRole('button').filter((b) => b.className.split(' ').includes('r'));
    fireEvent.click(rows[0]);
    expect(screen.queryByText('export_config')).toBeNull();
    expect(screen.queryByText('delete')).toBeNull();

    fireEvent.click(rows[1]);
    expect(screen.getByText('export_config')).toBeTruthy();
    expect(screen.getByText('delete')).toBeTruthy();
  });
});
