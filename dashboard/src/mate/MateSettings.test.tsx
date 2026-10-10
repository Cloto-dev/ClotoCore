import { fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata, McpServerInfo, McpServerStatus } from '../types';
import { MateSettings, serverSummary } from './MateSettings';

const m = vi.hoisted(() => ({
  remote: {
    servers: [] as McpServerInfo[],
    isLoading: false,
    error: null as string | null,
    refetch: vi.fn().mockResolvedValue(undefined),
  },
}));
vi.mock('../hooks/useMcpServers', () => ({ useMcpServers: () => m.remote }));
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
  });

  it("shows the partner's engine and memory server with their current state", () => {
    render(<MateSettings {...props()} />);
    expect(row('mate.engine_row').getByText('cerebras')).toBeInTheDocument();
    expect(row('mate.engine_row').getByText('mate.server_running')).toBeInTheDocument();
    expect(row('mate.memory_row').getByText('mate.server_error')).toBeInTheDocument();
    expect(row('mate.memory_row').getByText('exited')).toBeInTheDocument();
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

  it('says not set when the partner has no memory server', () => {
    render(<MateSettings {...props({ agent: { ...MIO, metadata: {} } })} />);
    expect(row('mate.memory_row').getByText('mate.server_unset')).toBeInTheDocument();
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

  it('opens the agent settings at the section each row names', () => {
    const p = props();
    render(<MateSettings {...p} />);
    fireEvent.click(row('mate.engine_row').getByRole('button', { name: 'mate.change' }));
    expect(p.onAgentSettings).toHaveBeenLastCalledWith('engine');
    fireEvent.click(row('mate.memory_row').getByRole('button', { name: 'mate.change' }));
    expect(p.onAgentSettings).toHaveBeenLastCalledWith('memory');
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
});
