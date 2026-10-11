import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMemoryTarget, AgentMetadata, McpServerInfo, McpServerStatus } from '../types';
import { MateSettings, serverSummary } from './MateSettings';

const m = vi.hoisted(() => ({
  remote: {
    servers: [] as McpServerInfo[],
    isLoading: false,
    error: null as string | null,
    refetch: vi.fn().mockResolvedValue(undefined),
  },
  api: { getAgentMemory: vi.fn() },
}));
vi.mock('../hooks/useMcpServers', () => ({ useMcpServers: () => m.remote }));
vi.mock('../hooks/useApi', () => ({ useApi: () => m.api }));
// The engine setup has its own tests; here it only has to be opened, closed and reported from.
vi.mock('./EngineSetup', () => ({
  EngineSetup: (p: { agent: AgentMetadata; onSet: () => void; onClose: () => void; onDetails: () => void }) => (
    <section aria-label="engine setup">
      <span>{p.agent.name}</span>
      <button type="button" onClick={p.onSet}>
        stub set
      </button>
      <button type="button" onClick={p.onClose}>
        stub close
      </button>
      <button type="button" onClick={p.onDetails}>
        stub details
      </button>
    </section>
  ),
}));
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, values?: Record<string, unknown>) => (values ? `${key} ${JSON.stringify(values)}` : key),
  }),
}));

function server(id: string, status: McpServerStatus, extra: Partial<McpServerInfo> = {}): McpServerInfo {
  return { id, command: 'x', args: [], status, tools: [], is_cloto_sdk: false, ...extra };
}

const MIO: AgentMetadata = {
  id: 'agent.mio',
  name: 'Mio',
  description: '',
  default_engine_id: 'cerebras',
  required_capabilities: [],
  enabled: true,
  last_seen: 0,
  status: 'online',
  metadata: { preferred_memory: 'cpersona' },
};

describe('serverSummary', () => {
  it('has nothing to say about a server the partner does not use', () => {
    expect(serverSummary(undefined, [])).toBeNull();
    expect(serverSummary('', [])).toBeNull();
  });

  it('does not call a server missing before the list has arrived, or when it could not be read', () => {
    expect(serverSummary('cerebras', null, 'checking')?.state).toBe('checking');
    expect(serverSummary('cerebras', null, 'unknown')?.state).toBe('unknown');
    expect(serverSummary('cerebras', [])?.state).toBe('missing');
  });

  it('reads the state from the server the kernel reports', () => {
    const cases: [McpServerStatus, string][] = [
      ['Connected', 'running'],
      ['Connecting', 'starting'],
      ['Restarting', 'starting'],
      ['Registered', 'stopped'],
      ['Draining', 'stopped'],
      ['Disconnected', 'stopped'],
      ['Error', 'error'],
    ];
    for (const [status, state] of cases) {
      expect(serverSummary('cerebras', [server('cerebras', status)])?.state, status).toBe(state);
    }
  });

  it("matches the configured id exactly, and passes the kernel's failure text through", () => {
    const list = [server('cerebras-old', 'Connected'), server('cerebras', 'Error', { status_message: 'spawn failed' })];
    expect(serverSummary('cerebras', list)).toEqual({ name: 'cerebras', state: 'error', message: 'spawn failed' });
  });

  it('names the server by its display name when it has one', () => {
    expect(serverSummary('cerebras', [server('cerebras', 'Connected', { display_name: 'Cerebras' })])?.name).toBe(
      'Cerebras',
    );
  });
});

function target(over: Partial<AgentMemoryTarget>): AgentMemoryTarget {
  return { plugin: null, server_id: null, allowed: false, ...over };
}

function props(over: Partial<Parameters<typeof MateSettings>[0]> = {}) {
  return {
    agent: MIO,
    connected: true,
    questions: 0,
    chatSize: 'side' as const,
    onChatSize: vi.fn(),
    glass: 'soft' as const,
    onGlass: vi.fn(),
    onPartner: vi.fn(),
    onReview: vi.fn(),
    onAgentSettings: vi.fn(),
    onAgentChanged: vi.fn(),
    onServers: vi.fn(),
    onAppSettings: vi.fn(),
    onBack: vi.fn(),
    ...over,
  };
}

function row(label: string) {
  const term = screen.getByText(label, { selector: 'dt' });
  return within(term.parentElement as HTMLElement);
}

describe('MateSettings', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    m.remote = {
      servers: [server('cerebras', 'Connected'), server('cpersona', 'Error', { status_message: 'exited' })],
      isLoading: false,
      error: null,
      refetch: vi.fn().mockResolvedValue(undefined),
    };
    m.api.getAgentMemory.mockResolvedValue(target({ server_id: 'cpersona', allowed: true }));
  });

  it("shows the partner's engine and the kernel's memory server with their current state", async () => {
    render(<MateSettings {...props()} />);
    expect(row('mate.engine_row').getByText('cerebras')).toBeInTheDocument();
    expect(row('mate.engine_row').getByText('mate.server_running')).toBeInTheDocument();
    expect(await row('mate.memory_row').findByText('mate.server_error')).toBeInTheDocument();
    expect(row('mate.memory_row').getByText('cpersona')).toBeInTheDocument();
    expect(row('mate.memory_row').getByText('exited')).toBeInTheDocument();
    expect(m.api.getAgentMemory).toHaveBeenCalledWith('agent.mio');
  });

  it("names the server the kernel uses, not the agent's preferred_memory", async () => {
    m.remote = { ...m.remote, servers: [server('cpersona', 'Connected'), server('other-memory', 'Connected')] };
    const agent = { ...MIO, metadata: { preferred_memory: 'other-memory' } };
    render(<MateSettings {...props({ agent })} />);
    expect(await row('mate.memory_row').findByText('cpersona')).toBeInTheDocument();
    expect(screen.queryByText('other-memory')).not.toBeInTheDocument();
  });

  it('says the partner cannot use memory only when the kernel refuses it', async () => {
    m.api.getAgentMemory.mockResolvedValue(target({ server_id: 'cpersona', allowed: false }));
    const { unmount } = render(<MateSettings {...props()} />);
    expect(await row('mate.memory_row').findByText('mate.memory_not_allowed')).toBeInTheDocument();
    unmount();
    m.api.getAgentMemory.mockResolvedValue(target({ server_id: 'cpersona', allowed: true }));
    render(<MateSettings {...props()} />);
    expect(await row('mate.memory_row').findByText('cpersona')).toBeInTheDocument();
    expect(screen.queryByText('mate.memory_not_allowed')).not.toBeInTheDocument();
  });

  it('says no server has memory features when the kernel names none', async () => {
    m.api.getAgentMemory.mockResolvedValue(target({}));
    render(<MateSettings {...props()} />);
    expect(await row('mate.memory_row').findByText('mate.memory_none')).toBeInTheDocument();
    expect(screen.queryByText('mate.memory_not_allowed')).not.toBeInTheDocument();
  });

  it('names a memory plugin built into the kernel as running', async () => {
    m.api.getAgentMemory.mockResolvedValue(target({ plugin: 'builtin-memory', allowed: null }));
    render(<MateSettings {...props()} />);
    expect(await row('mate.memory_row').findByText('builtin-memory')).toBeInTheDocument();
    expect(row('mate.memory_row').getByText('mate.server_running')).toBeInTheDocument();
  });

  it('says checking until the memory answer arrives, and unavailable when it fails', async () => {
    m.api.getAgentMemory.mockReturnValue(new Promise(() => {}));
    const { unmount } = render(<MateSettings {...props()} />);
    expect(row('mate.memory_row').getByText('mate.server_checking')).toBeInTheDocument();
    unmount();
    m.api.getAgentMemory.mockRejectedValue(new Error('Failed to fetch memory target'));
    render(<MateSettings {...props()} />);
    expect(await row('mate.memory_row').findByText('mate.server_unknown')).toBeInTheDocument();
  });

  it('says the engine cannot answer only when it is missing, stopped or failed', () => {
    m.remote = { ...m.remote, servers: [server('cpersona', 'Connected')] };
    const { rerender } = render(<MateSettings {...props()} />);
    expect(row('mate.engine_row').getByText('mate.engine_cannot_answer')).toBeInTheDocument();
    m.remote = { ...m.remote, servers: [server('cerebras', 'Connected')] };
    rerender(<MateSettings {...props({ questions: 1 })} />);
    expect(screen.queryByText('mate.engine_cannot_answer')).not.toBeInTheDocument();
    m.remote = { ...m.remote, servers: [], isLoading: true };
    rerender(<MateSettings {...props({ questions: 2 })} />);
    expect(screen.queryByText('mate.engine_cannot_answer')).not.toBeInTheDocument();
  });

  it('says checking while the server list is loading, not that the servers are missing', () => {
    m.remote = { ...m.remote, servers: [], isLoading: true };
    render(<MateSettings {...props()} />);
    expect(row('mate.engine_row').getByText('mate.server_checking')).toBeInTheDocument();
    expect(screen.queryByText('mate.server_missing')).not.toBeInTheDocument();
  });

  it('says the state is unavailable when the list could not be read', () => {
    m.remote = { ...m.remote, servers: [], error: 'Failed to connect' };
    render(<MateSettings {...props()} />);
    expect(row('mate.engine_row').getByText('mate.server_unknown')).toBeInTheDocument();
    expect(screen.queryByText('mate.server_missing')).not.toBeInTheDocument();
  });

  it('marks the stored display choices and reports a new one', () => {
    const p = props({ chatSize: 'expanded', glass: 'still' });
    render(<MateSettings {...p} />);
    expect(screen.getByRole('button', { name: 'mate.chat_size_expanded' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('button', { name: 'mate.chat_size_side' })).toHaveAttribute('aria-pressed', 'false');
    expect(screen.getByRole('button', { name: 'mate.glass_still' })).toHaveAttribute('aria-pressed', 'true');
    fireEvent.click(screen.getByRole('button', { name: 'mate.chat_size_side' }));
    expect(p.onChatSize).toHaveBeenCalledWith('side');
    fireEvent.click(screen.getByRole('button', { name: 'mate.glass_crisp' }));
    expect(p.onGlass).toHaveBeenCalledWith('crisp');
  });

  it('offers Change for a working engine and Set up for one that cannot answer', () => {
    const { rerender } = render(<MateSettings {...props()} />);
    expect(row('mate.engine_row').getByRole('button', { name: 'mate.change' })).toBeInTheDocument();
    m.remote = { ...m.remote, servers: [] };
    rerender(<MateSettings {...props({ questions: 1 })} />);
    expect(row('mate.engine_row').getByRole('button', { name: 'mate.set_up' })).toBeInTheDocument();
    rerender(<MateSettings {...props({ agent: { ...MIO, default_engine_id: '' } })} />);
    expect(row('mate.engine_row').getByRole('button', { name: 'mate.set_up' })).toBeInTheDocument();
  });

  it('opens the engine setup in the panel, and comes back from it', () => {
    const p = props();
    render(<MateSettings {...p} />);
    fireEvent.click(row('mate.engine_row').getByRole('button', { name: 'mate.change' }));
    expect(screen.getByRole('region', { name: 'engine setup' })).toHaveTextContent('Mio');
    expect(screen.queryByText('mate.engine_row', { selector: 'dt' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'stub close' }));
    expect(screen.getByText('mate.engine_row', { selector: 'dt' })).toBeInTheDocument();
    expect(p.onAgentSettings).not.toHaveBeenCalled();
  });

  it('reads the partner and the servers again once the engine is set', () => {
    const p = props();
    render(<MateSettings {...p} />);
    fireEvent.click(row('mate.engine_row').getByRole('button', { name: 'mate.change' }));
    fireEvent.click(screen.getByRole('button', { name: 'stub set' }));
    expect(p.onAgentChanged).toHaveBeenCalledTimes(1);
    expect(m.remote.refetch).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: 'stub details' }));
    expect(p.onAgentSettings).toHaveBeenCalledWith('engine');
  });

  it('returns to the panel when the partner changes during the engine setup', () => {
    const { rerender } = render(<MateSettings {...props()} />);
    fireEvent.click(row('mate.engine_row').getByRole('button', { name: 'mate.change' }));
    rerender(<MateSettings {...props({ agent: { ...MIO, id: 'agent.rin', name: 'Rin' } })} />);
    expect(screen.queryByRole('region', { name: 'engine setup' })).not.toBeInTheDocument();
    expect(screen.getByText('mate.engine_row', { selector: 'dt' })).toBeInTheDocument();
  });

  it('offers nothing to change on the memory row, and opens the partner settings from its row', () => {
    const p = props();
    render(<MateSettings {...p} />);
    // The memory server is not the agent's to choose.
    expect(row('mate.memory_row').queryByRole('button')).not.toBeInTheDocument();
    fireEvent.click(row('mate.appearance_row').getByRole('button', { name: 'mate.change' }));
    expect(p.onPartner).toHaveBeenCalledTimes(1);
  });

  it('offers review only when approvals are waiting, and not while disconnected', () => {
    const { rerender } = render(<MateSettings {...props()} />);
    expect(row('mate.approvals_row').getByText('mate.approvals_none')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'mate.review_question' })).not.toBeInTheDocument();
    const p = props({ questions: 2 });
    rerender(<MateSettings {...p} />);
    expect(row('mate.approvals_row').getByText('mate.approvals_count {"n":2}')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'mate.review_question' }));
    expect(p.onReview).toHaveBeenCalledTimes(1);
    rerender(<MateSettings {...props({ questions: 2, connected: false })} />);
    expect(screen.getByRole('button', { name: 'mate.review_question' })).toBeDisabled();
  });

  it('shows the display and app sections without a partner, and no partner rows', () => {
    const p = props({ agent: null });
    render(<MateSettings {...p} />);
    expect(screen.queryByText('mate.engine_row')).not.toBeInTheDocument();
    expect(screen.queryByText('mate.approvals_row')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'mate.chat_size_side' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'mate.app_settings' }));
    expect(p.onAppSettings).toHaveBeenCalledTimes(1);
  });

  it('shows whether the kernel is connected', () => {
    const { rerender } = render(<MateSettings {...props()} />);
    expect(row('mate.kernel_row').getByText('mate.kernel_connected')).toBeInTheDocument();
    rerender(<MateSettings {...props({ connected: false })} />);
    expect(row('mate.kernel_row').getByText('mate.state_offline')).toBeInTheDocument();
  });

  it('reads the server list again when the kernel comes back, and only then', () => {
    // Opening the panel while connected leaves the first read to the list hook itself.
    const opened = render(<MateSettings {...props({ connected: true })} />);
    expect(m.remote.refetch).not.toHaveBeenCalled();
    opened.unmount();
    const { rerender } = render(<MateSettings {...props({ connected: false })} />);
    expect(m.remote.refetch).not.toHaveBeenCalled();
    rerender(<MateSettings {...props({ connected: true })} />);
    expect(m.remote.refetch).toHaveBeenCalledTimes(1);
    rerender(<MateSettings {...props({ connected: true, questions: 1 })} />);
    expect(m.remote.refetch).toHaveBeenCalledTimes(1);
  });

  it('asks for the memory answer again when the kernel comes back, and only then', async () => {
    const { rerender } = render(<MateSettings {...props({ connected: false })} />);
    await waitFor(() => expect(m.api.getAgentMemory).toHaveBeenCalledTimes(1));
    await act(async () => rerender(<MateSettings {...props({ connected: true })} />));
    expect(m.api.getAgentMemory).toHaveBeenCalledTimes(2);
    await act(async () => rerender(<MateSettings {...props({ connected: true, questions: 1 })} />));
    expect(m.api.getAgentMemory).toHaveBeenCalledTimes(2);
  });

  it('keeps the answer for the partner on screen when an earlier request finishes late', async () => {
    let finishFirst: (value: AgentMemoryTarget) => void = () => {};
    m.api.getAgentMemory
      .mockReturnValueOnce(new Promise((resolve) => (finishFirst = resolve)))
      .mockResolvedValueOnce(target({ server_id: 'second-memory', allowed: true }));
    const { rerender } = render(<MateSettings {...props()} />);
    rerender(<MateSettings {...props({ agent: { ...MIO, id: 'agent.rin', name: 'Rin' } })} />);
    expect(await row('mate.memory_row').findByText('second-memory')).toBeInTheDocument();
    await act(async () => finishFirst(target({ server_id: 'first-memory', allowed: true })));
    expect(screen.queryByText('first-memory')).not.toBeInTheDocument();
  });
});
