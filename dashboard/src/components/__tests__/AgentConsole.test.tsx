import { act, fireEvent, render, screen, within } from '@testing-library/react';
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
  stopResponse: vi.fn(),
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
// The typewriter, without the typing. By default it never finishes, which keeps
// a reply "arriving" for the tests that look at that state; a test that needs
// the room free again sets `typewriter.finishes`.
const typewriter = vi.hoisted(() => ({ finishes: false }));
vi.mock('../TypewriterMessage', () => ({
  TypewriterMessage: ({ text, onComplete }: { text: string; onComplete?: () => void }) => {
    if (typewriter.finishes) queueMicrotask(() => onComplete?.());
    return <div>{text}</div>;
  },
}));
vi.mock('../ContentBlockView', () => ({
  MessageContent: ({ content }: { content: Array<{ type: string; text?: string }> }) => (
    <p>{content.map((b) => b.text).join('')}</p>
  ),
}));
// The report itself is the kernel's; what is asked here is what the room hands it.
vi.mock('../DiagnosticsModal', () => ({
  DiagnosticsModal: ({ context, message }: { context?: string; message?: string }) => (
    <div role="dialog">{`${context} / ${message}`}</div>
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
  typewriter.finishes = false;
  api.getAgentAccess.mockReset().mockResolvedValue({ entries: [] });
  api.getChatMessages.mockReset().mockResolvedValue({ messages: [], has_more: false });
  api.postChat.mockReset().mockResolvedValue(undefined);
  api.postChatMessage.mockReset().mockResolvedValue({ id: 'x', created_at: 0 });
  api.retryResponse.mockReset().mockResolvedValue({ retry_id: 'r' });
  api.stopResponse.mockReset().mockResolvedValue({ stopped: true });
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
    // Noon today, not the present moment: between 00:00 and 01:00 "an hour ago"
    // is yesterday, and the two turns below stop being today's and yesterday's.
    const now = new Date().setHours(12, 0, 0, 0);
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

  it('asks the kernel to stop the reply it was waiting for', async () => {
    draw();
    await screen.findByText('console.remark');
    const stopped = await send('slow question');
    fireEvent.click(screen.getByRole('button', { name: 'chat_input.stop' }));
    expect(api.stopResponse).toHaveBeenCalledWith('agent.a', stopped);
  });

  it('shows the reply after all when the kernel says it had already finished', async () => {
    api.stopResponse.mockResolvedValue({ stopped: false });
    draw();
    await screen.findByText('console.remark');
    const finished = await send('quick question');
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'chat_input.stop' }));
    });
    // The reply is stored; holding it back would hide it until a reload.
    reply(finished, 'it was already written');
    expect(await screen.findByText('it was already written')).toBeTruthy();
  });

  it('keeps the reply held back when the stop call fails', async () => {
    api.stopResponse.mockRejectedValue(new Error('down'));
    draw();
    await screen.findByText('console.remark');
    const unsure = await send('question');
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'chat_input.stop' }));
    });
    reply(unsure, 'should not be drawn');
    expect(screen.queryByText('should not be drawn')).toBeNull();
  });

  it('ends the wait when the reply is stopped from another window', async () => {
    draw();
    await screen.findByText('console.remark');
    const elsewhere = await send('question');
    expect(screen.getByRole('button', { name: 'chat_input.stop' })).toBeTruthy();
    act(() => {
      stream.handler?.({ type: 'ResponseStopped', data: { agent_id: 'agent.b', source_message_id: elsewhere } });
    });
    // Another agent's stop is not this one's.
    expect(screen.getByRole('button', { name: 'chat_input.stop' })).toBeTruthy();
    act(() => {
      stream.handler?.({ type: 'ResponseStopped', data: { agent_id: 'agent.a', source_message_id: elsewhere } });
    });
    expect(screen.getByRole('button', { name: 'chat_input.send' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'chat_input.stop' })).toBeNull();
  });
});

describe('a turn the engine could not produce', () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  beforeEach(() => {
    writeText.mockClear();
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  });

  it("is drawn where the reply would be, with the engine's words and what the turn needs", async () => {
    const now = new Date().setHours(12, 0, 0, 0);
    api.getChatMessages.mockResolvedValue({
      messages: [
        { ...msg('u1-resp', 'agent', '[Error] Provider returned 402', now), parent_id: 'u1' },
        msg('u1', 'user', '[Error] is what I typed', now - 1000),
      ],
      has_more: false,
    });
    draw();

    const words = await screen.findByText('Provider returned 402');
    expect(screen.getByText('console.reply_failed')).toBeTruthy();
    // The kernel's marker is not part of what went wrong.
    expect(screen.queryByText(/Provider returned 402/, { selector: '.err' })?.textContent).toBe(
      'Provider returned 402',
    );
    // Not the agent speaking: its own form, and no reading aloud.
    expect(words.closest('.msg')?.classList.contains('fail')).toBe(true);
    expect(screen.queryByRole('button', { name: 'console.read_aloud' })).toBeNull();
    // The user's own words are theirs, whatever they start with.
    expect(screen.getByText('[Error] is what I typed').closest('.me')).toBeTruthy();

    const turn = within(words.closest('.msg') as HTMLElement);
    fireEvent.click(turn.getByRole('button', { name: 'console.copy' }));
    expect(writeText).toHaveBeenCalledWith('Provider returned 402');

    fireEvent.click(turn.getByRole('button', { name: 'console.report_details' }));
    expect(screen.getByRole('dialog').textContent).toBe('console.report_context / Provider returned 402');

    fireEvent.click(turn.getByRole('button', { name: 'console.retry' }));
    await vi.waitFor(() => expect(api.retryResponse).toHaveBeenCalledWith('agent.a', 'u1'));
  });

  it('arrives in that form, and is never typed out as if the agent were saying it', async () => {
    draw();
    await screen.findByText('console.remark');
    const id = await send('hello');
    reply(id, '[Error] engine down');
    expect(await screen.findByText('console.reply_failed')).toBeTruthy();
    expect(screen.getByText('engine down')).toBeTruthy();
    // The typewriter (which never finishes here) was not handed the text.
    expect(screen.queryByText('[Error] engine down')).toBeNull();
  });
});

describe('a message that could not be sent', () => {
  async function failToSend(text: string): Promise<HTMLElement> {
    draw();
    await screen.findByText('console.remark');
    api.postChat.mockRejectedValueOnce(new Error('kernel unreachable'));
    await send(text);
    const turn = (await screen.findByText(text)).closest('.me') as HTMLElement;
    await vi.waitFor(() => expect(turn.classList.contains('failed')).toBe(true));
    return turn;
  }

  it('stays where it was written, says why, and is sent again from there', async () => {
    const turn = await failToSend('are you there');
    expect(within(turn).getByText(/console\.send_failed kernel unreachable/)).toBeTruthy();

    fireEvent.click(within(turn).getByRole('button', { name: 'console.send_again' }));
    await vi.waitFor(() => expect(api.postChat).toHaveBeenCalledTimes(2));
    expect((api.postChat.mock.calls[1][0] as { content: string }).content).toBe('are you there');
    // Sent once more, not twice over: one copy, and it is no longer marked.
    expect(screen.getAllByText('are you there')).toHaveLength(1);
    expect(document.querySelector('.me.failed')).toBeNull();
  });

  it('is edited into a new message in its place, not a branch of one the kernel never had', async () => {
    const turn = await failToSend('first try');
    fireEvent.click(within(turn).getByRole('button', { name: 'console.edit_message' }));
    fireEvent.change(box(), { target: { value: 'second try' } });
    fireEvent.keyDown(box(), { key: 'Enter' });
    await vi.waitFor(() => expect(api.postChat).toHaveBeenCalledTimes(2));
    const sent = api.postChat.mock.calls[1][0] as { content: string; metadata: Record<string, string> };
    expect(sent.content).toBe('second try');
    expect(sent.metadata.parent_id).toBeUndefined();
    expect(screen.queryByText('first try')).toBeNull();
  });
});

describe('a console mounted on the new chat', () => {
  const first = { blocks: [{ type: 'text' as const, text: 'hello' }], rawText: 'hello', engineOverride: null };

  it('asks for no history, creates the conversation, and sends the first message into it', async () => {
    const onFirstMessage = vi.fn().mockResolvedValue('made-1');
    render(
      <AgentConsole
        agent={agent}
        conversationId={null}
        onFirstMessage={onFirstMessage}
        initialSend={first}
        onBack={vi.fn()}
      />,
    );
    await vi.waitFor(() => expect(api.postChat).toHaveBeenCalledTimes(1));
    expect(onFirstMessage).toHaveBeenCalledTimes(1);
    const dispatched = api.postChat.mock.calls[0][0] as { content: string; metadata: { conversation_id: string } };
    expect(dispatched.content).toBe('hello');
    expect(dispatched.metadata.conversation_id).toBe('made-1');
    // Without a conversation id the history endpoint answers with everything
    // the agent was ever sent: a draft must not ask.
    expect(api.getChatMessages).not.toHaveBeenCalled();
    expect(screen.getByText('hello')).toBeTruthy();
  });

  it('creates once: the second message goes into the conversation the first one made', async () => {
    typewriter.finishes = true;
    const onFirstMessage = vi.fn().mockResolvedValue('made-1');
    render(
      <AgentConsole
        agent={agent}
        conversationId={null}
        onFirstMessage={onFirstMessage}
        initialSend={first}
        onBack={vi.fn()}
      />,
    );
    await vi.waitFor(() => expect(api.postChat).toHaveBeenCalledTimes(1));
    const firstId = (api.postChat.mock.calls[0][0] as { id: string }).id;
    reply(firstId, 'hi there');
    // The room takes a new message once the reply has finished arriving.
    await vi.waitFor(
      () => {
        fireEvent.change(box(), { target: { value: 'and again' } });
        fireEvent.keyDown(box(), { key: 'Enter' });
        expect(api.postChat).toHaveBeenCalledTimes(2);
      },
      { timeout: 4000 },
    );
    expect(onFirstMessage).toHaveBeenCalledTimes(1);
    const second = api.postChat.mock.calls[1][0] as { metadata: { conversation_id: string } };
    expect(second.metadata.conversation_id).toBe('made-1');
  });

  it('says so, and sends nothing, when the conversation cannot be created', async () => {
    const onFirstMessage = vi.fn().mockRejectedValue(new Error('kernel said no'));
    render(
      <AgentConsole
        agent={agent}
        conversationId={null}
        onFirstMessage={onFirstMessage}
        initialSend={first}
        onBack={vi.fn()}
      />,
    );
    await screen.findByText(/kernel said no/);
    expect(api.postChat).not.toHaveBeenCalled();
  });
});
