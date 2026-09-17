import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata } from '../../types';

const apiFns = vi.hoisted(() => ({
  apiKey: 'k',
  getNotifications: vi.fn(),
  markNotificationRead: vi.fn(),
}));
const fixed = vi.hoisted(() => ({
  servers: { servers: [], refetch: () => Promise.resolve() },
  agentContext: { processingAgentIds: new Set<string>() },
  conversations: {
    openFor: () => 'c1',
    resolveOpen: () => Promise.resolve(null),
    draft: null as { key: string; agentId: string | null; first?: unknown } | null,
    commitDraft: () => Promise.resolve('c1'),
    mountKeyFor: (a: string, c: string) => `${a}:${c}`,
    leaveDraft: () => {},
  },
}));

vi.mock('react-router-dom', () => ({ useNavigate: () => vi.fn(), useLocation: () => ({ pathname: '/' }) }));
vi.mock('../../hooks/useApi', () => ({ useApi: () => apiFns }));
vi.mock('../../hooks/useEventStream', () => ({ useEventStream: () => {} }));
vi.mock('../../hooks/useMcpServers', () => ({ useMcpServers: () => fixed.servers }));
vi.mock('../../contexts/AgentContext', () => ({ useAgentContext: () => fixed.agentContext }));
vi.mock('../../contexts/ConversationContext', () => ({ useConversations: () => fixed.conversations }));
vi.mock('../AgentConsole', () => ({ AgentConsole: () => <div>console</div> }));
vi.mock('../agents/AgentRoster', () => ({ AgentRoster: () => <div>roster</div> }));
vi.mock('../NewChatScreen', () => ({ NewChatScreen: () => <div>new-chat</div> }));

import { AgentTerminal } from '../AgentTerminal';

const AGENT: AgentMetadata = {
  id: 'agent.a',
  name: 'A',
  description: '',
  required_capabilities: [],
  enabled: true,
  last_seen: 0,
  status: 'online',
  metadata: {},
};

const WAITING = {
  item_id: 'q1',
  kind: 'approval',
  severity: 'warning',
  agent_id: 'agent.a',
  title: 'May I?',
  body: null,
  created_at: '2026-09-17T00:00:00Z',
  read_at: null,
  resolved_at: null,
  decision: null,
  blocking: true,
  metadata: null,
};

beforeEach(() => {
  vi.clearAllMocks();
  fixed.conversations.draft = null;
  apiFns.getNotifications.mockResolvedValue([WAITING]);
  apiFns.markNotificationRead.mockResolvedValue(undefined);
});

describe('the agent route', () => {
  // The conversation is reached from the sidebar as often as from the roster,
  // so the route — not the roster — is what reads the waiting question.
  it('reads what was waiting once the conversation is what is on screen', async () => {
    render(<AgentTerminal agents={[AGENT]} selectedAgent={AGENT} onSelectAgent={() => {}} onRefresh={() => {}} />);
    expect(screen.getByText('console')).toBeTruthy();
    await waitFor(() => expect(apiFns.markNotificationRead).toHaveBeenCalledWith('q1'));
  });

  it('leaves it unread while the roster is what is on screen', async () => {
    render(<AgentTerminal agents={[AGENT]} selectedAgent={null} onSelectAgent={() => {}} onRefresh={() => {}} />);
    expect(screen.getByText('roster')).toBeTruthy();
    await Promise.resolve();
    expect(apiFns.markNotificationRead).not.toHaveBeenCalled();
  });

  it("shows the new chat while someone is being chosen, and reads nobody's questions for turning past them", async () => {
    fixed.conversations.draft = { key: 'draft:1', agentId: 'agent.a' };
    render(<AgentTerminal agents={[AGENT]} selectedAgent={AGENT} onSelectAgent={() => {}} onRefresh={() => {}} />);
    expect(screen.getByText('new-chat')).toBeTruthy();
    expect(screen.queryByText('console')).toBeNull();
    await Promise.resolve();
    expect(apiFns.markNotificationRead).not.toHaveBeenCalled();
  });

  it('hands over to the console once the first message is written', () => {
    fixed.conversations.draft = {
      key: 'draft:1',
      agentId: 'agent.a',
      first: { blocks: [], rawText: 'hello', engineOverride: null },
    };
    render(<AgentTerminal agents={[AGENT]} selectedAgent={AGENT} onSelectAgent={() => {}} onRefresh={() => {}} />);
    expect(screen.getByText('console')).toBeTruthy();
    expect(screen.queryByText('new-chat')).toBeNull();
  });
});
