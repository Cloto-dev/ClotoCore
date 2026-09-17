import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata, ChatMessage } from '../../types';

// Echo i18n keys so the assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k, i18n: { language: 'en' } }),
}));

const api = vi.hoisted(() => ({
  apiKey: 'k',
  getAgentAccess: vi.fn(),
  getChatMessages: vi.fn(),
  postChat: vi.fn(),
  postChatMessage: vi.fn(),
  retryResponse: vi.fn(),
  getAgentLastUsage: vi.fn(),
  getAvatarUrl: vi.fn(() => ''),
  getNotifications: vi.fn(),
  approveCommand: vi.fn(),
  trustCommand: vi.fn(),
  denyCommand: vi.fn(),
}));
const stream = vi.hoisted(() => ({ handler: null as null | ((event: Record<string, unknown>) => void) }));
const conversations = vi.hoisted(() => ({ open: vi.fn(), refresh: vi.fn() }));

vi.mock('../../hooks/useApi', () => ({ useApi: () => api }));
vi.mock('../../hooks/useEventStream', () => ({
  useEventStream: (_url: string, handler: (event: Record<string, unknown>) => void) => {
    stream.handler = handler;
  },
}));
// Stable across renders: the console re-resolves its engines whenever the
// server list changes identity, so a fresh array per render would loop.
const mcp = vi.hoisted(() => ({ servers: [] }));
vi.mock('../../hooks/useMcpServers', () => ({ useMcpServers: () => mcp }));
vi.mock('../../vrm/useGazeBroadcast', () => ({ useGazeBroadcast: () => {} }));
vi.mock('../../lib/tauri', () => ({ openVrmWindow: vi.fn(), isTauri: false }));
vi.mock('../../contexts/ActionsContext', () => ({
  useActionsContext: () => ({
    addArtifact: vi.fn(),
    clearArtifacts: vi.fn(),
    isOpen: false,
    closePanel: vi.fn(),
    openPanel: vi.fn(),
    activeCategory: 'dialogues',
    setActiveCategory: vi.fn(),
    hasDialogues: false,
    hasExternalActions: false,
    hasConsensus: false,
    artifacts: [],
    activeArtifactIndex: 0,
    setActiveArtifactIndex: vi.fn(),
    dialogues: [],
    externalActions: [],
    consensusRounds: [],
    unreadDialogueCount: 0,
    unreadExternalCount: 0,
    unreadConsensusCount: 0,
    totalCount: 0,
  }),
}));
vi.mock('../../contexts/AgentContext', () => ({
  useAgentContext: () => ({ agents: [], setSelectedAgentId: vi.fn() }),
}));
vi.mock('../../contexts/ConversationContext', () => ({
  useConversations: () => ({ conversations: [], open: conversations.open, refresh: conversations.refresh }),
}));
vi.mock('../../contexts/UserIdentityContext', () => ({
  useUserIdentity: () => ({ identity: { id: 'u1', name: 'me' } }),
}));
vi.mock('../ActionsPanel', () => ({ ActionsPanel: () => null }));
vi.mock('../../lib/agentIdentity', () => ({
  AgentIcon: () => null,
  agentColor: () => 'hsl(190 70% 58%)',
}));
// The reply's text, without the typewriter's timers and markdown pipeline.
vi.mock('../TypewriterMessage', () => ({
  TypewriterMessage: ({ text }: { text: string }) => <div>{text}</div>,
}));
vi.mock('../ContentBlockView', () => ({
  MessageContent: ({ content }: { content: Array<{ type: string; text?: string }> }) => (
    <p>{content.map((b) => b.text).join('')}</p>
  ),
}));

import { inlineSnapshot, resetInline } from '../../lib/inlineApprovals';
import { AgentConsole } from '../AgentConsole';

const agent: AgentMetadata = {
  id: 'agent.a',
  name: 'Sapphy',
  description: '',
  enabled: true,
  metadata: {},
} as unknown as AgentMetadata;

function msg(id: string, source: 'user' | 'agent', text: string, created_at: number): ChatMessage {
  return { id, agent_id: 'agent.a', user_id: 'u1', source, content: [{ type: 'text', text }], created_at };
}

const HOUR = 60 * 60 * 1000;

function draw() {
  return render(<AgentConsole agent={agent} conversationId="c1" onBack={vi.fn()} />);
}

const box = () => screen.getByRole('textbox') as HTMLTextAreaElement;

async function send(text: string): Promise<string> {
  fireEvent.change(box(), { target: { value: text } });
  fireEvent.keyDown(box(), { key: 'Enter' });
  await vi.waitFor(() => expect(api.postChat).toHaveBeenCalled());
  const dispatched = api.postChat.mock.calls[api.postChat.mock.calls.length - 1][0] as { id: string };
  return dispatched.id;
}

function reply(sourceId: string, content: string) {
  act(() => {
    stream.handler?.({
      type: 'ThoughtResponse',
      data: { agent_id: 'agent.a', source_message_id: sourceId, content },
    });
  });
}

beforeEach(() => {
  stream.handler = null;
  api.getAgentAccess.mockReset().mockResolvedValue({ entries: [] });
  api.getChatMessages.mockReset().mockResolvedValue({ messages: [], has_more: false });
  api.postChat.mockReset().mockResolvedValue(undefined);
  api.postChatMessage.mockReset().mockResolvedValue({ id: 'x', created_at: 0 });
  api.retryResponse.mockReset().mockResolvedValue({ retry_id: 'r' });
  api.getAgentLastUsage.mockReset().mockResolvedValue({ usage: null });
  api.getNotifications.mockReset().mockResolvedValue([]);
  api.approveCommand.mockReset().mockResolvedValue(undefined);
  resetInline();
  conversations.open.mockReset();
  conversations.refresh.mockReset();
  sessionStorage.clear();
});

describe('the living room', () => {
  it('draws the day between days, a clock beside each turn, and the actions of each side', async () => {
    const now = Date.now();
    // Newest first, as the API returns them.
    api.getChatMessages.mockResolvedValue({
      messages: [msg('a1', 'agent', 'the reply', now - HOUR), msg('u1', 'user', 'the question', now - 25 * HOUR)],
      has_more: false,
    });
    draw();
    await screen.findByText('the reply');
    expect(screen.getByText('console.day_yesterday')).toBeTruthy();
    expect(screen.getByText('console.day_today')).toBeTruthy();
    expect(screen.getAllByText(/^\d\d:\d\d$/)).toHaveLength(2);
    // The user's turn: edit and copy. The agent's: copy, again, read aloud.
    expect(screen.getByRole('button', { name: 'console.edit_message' })).toBeTruthy();
    expect(screen.getAllByRole('button', { name: 'console.copy' })).toHaveLength(2);
    expect(screen.getByRole('button', { name: 'console.retry' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'console.read_aloud' })).toBeTruthy();
  });

  it('is the empty room when nothing has been said, and the conversation once something has', async () => {
    draw();
    expect(await screen.findByText('console.remark')).toBeTruthy();
    const id = await send('hello');
    expect(screen.queryByText('console.remark')).toBeNull();
    reply(id, 'hi there');
    expect(await screen.findByText('hi there')).toBeTruthy();
  });

  it("asks the agent's question inside the room, keeps the deck off it, and answers it here", async () => {
    draw();
    await screen.findByText('console.remark');
    act(() => {
      stream.handler?.({
        type: 'CommandApprovalRequested',
        data: {
          approval_id: 'ap-1',
          agent_id: 'agent.a',
          commands: [{ command: 'systemctl --user restart cloto-cron.service', command_name: 'systemctl' }],
        },
      });
    });
    expect(screen.getByText('systemctl --user restart cloto-cron.service')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'ask.go_ahead' })).toBeTruthy();
    // The room said so, so the deck will not ask the same question over it.
    expect(inlineSnapshot().has('ap-1')).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: 'ask.just_once' }));
    await vi.waitFor(() => expect(api.approveCommand).toHaveBeenCalledWith('ap-1'));
    await vi.waitFor(() => expect(screen.queryByText('systemctl --user restart cloto-cron.service')).toBeNull());
    expect(inlineSnapshot().has('ap-1')).toBe(false);
  });

  it("does not ask another agent's question in this room", async () => {
    draw();
    await screen.findByText('console.remark');
    act(() => {
      stream.handler?.({
        type: 'CommandApprovalRequested',
        data: { approval_id: 'ap-2', agent_id: 'agent.b', commands: [{ command: 'ls', command_name: 'ls' }] },
      });
    });
    expect(screen.queryByText('ls')).toBeNull();
    expect(inlineSnapshot().has('ap-2')).toBe(false);
  });

  it('stop keeps what was shown, says so, and does not draw the reply that still arrives', async () => {
    draw();
    await screen.findByText('console.remark');
    const stopped = await send('slow question');
    expect(screen.getByRole('button', { name: 'chat_input.stop' })).toBeTruthy();
    // A chunk arrives, then the user stops waiting.
    act(() => {
      stream.handler?.({
        type: 'AgentTokenStream',
        data: { agent_id: 'agent.a', source_message_id: stopped, delta: 'the beginning' },
      });
    });
    expect(screen.getByText('the beginning')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'chat_input.stop' }));
    expect(screen.getByText('console.stopped')).toBeTruthy();
    expect(screen.getByText('the beginning')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'chat_input.send' })).toBeTruthy();

    // The rest of that reply is not drawn — neither its chunks nor its end.
    act(() => {
      stream.handler?.({
        type: 'AgentTokenStream',
        data: { agent_id: 'agent.a', source_message_id: stopped, delta: ' and the rest' },
      });
    });
    reply(stopped, 'the beginning and the rest');
    expect(screen.queryByText('the beginning and the rest')).toBeNull();
    expect(screen.queryByText(/and the rest/)).toBeNull();

    // The next question is answered as usual.
    const next = await send('next question');
    reply(next, 'a fresh reply');
    expect(await screen.findByText('a fresh reply')).toBeTruthy();
  });
});
