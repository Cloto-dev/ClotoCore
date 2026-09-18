import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata, Conversation } from '../../types';

vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (k: string) => k }) }));
const api = vi.hoisted(() => ({ getAgentAccess: vi.fn() }));
vi.mock('../../hooks/useApi', () => ({ useApi: () => api }));
const servers = vi.hoisted(() => ({ servers: [] }));
vi.mock('../../hooks/useMcpServers', () => ({ useMcpServers: () => servers }));
vi.mock('../../lib/agentIdentity', () => ({ AgentIcon: () => <span>face</span> }));

const agentCtx = vi.hoisted(() => ({ agents: [] as AgentMetadata[], refetchAgents: vi.fn() }));
vi.mock('../../contexts/AgentContext', () => ({ useAgentContext: () => agentCtx }));
const convCtx = vi.hoisted(() => ({
  conversations: [] as Conversation[],
  draft: null as { key: string; agentId: string | null } | null,
  setDraftAgent: vi.fn(),
  sendDraft: vi.fn(),
}));
vi.mock('../../contexts/ConversationContext', () => ({ useConversations: () => convCtx }));

// The composer is the real one's contract, reduced to what this screen decides.
vi.mock('../ChatInputBar', () => ({
  ChatInputBar: (p: {
    disabled?: boolean;
    agentId?: string;
    onSend: (b: unknown[], t: string, e: null) => void;
    invitation?: { label: string; placeholder: string; onAccept: (name: string) => void };
  }) => (
    <>
      <button type="button" data-agent={p.agentId ?? ''} disabled={p.disabled} onClick={() => p.onSend([], 'hi', null)}>
        composer
      </button>
      {p.invitation && (
        <button type="button" title={p.invitation.placeholder} onClick={() => p.invitation?.onAccept('Mira')}>
          invitation:{p.invitation.label}
        </button>
      )}
    </>
  ),
}));
const created = vi.hoisted(() => ({ faceProblem: null as string | null }));
vi.mock('../agents/CreateAgentModal', () => ({
  CreateAgentModal: ({
    initialName,
    onCreated,
  }: {
    initialName?: string;
    onCreated: (c: { name: string; id: string | null; faceProblem: string | null }) => void;
  }) => (
    <button
      type="button"
      onClick={() => onCreated({ name: 'Newcomer', id: 'agent.newcomer', faceProblem: created.faceProblem })}
    >
      create-modal
      {initialName ? <span>named:{initialName}</span> : null}
    </button>
  ),
}));

import { NewChatScreen } from '../NewChatScreen';

const agent = (id: string, over: Partial<AgentMetadata> = {}): AgentMetadata =>
  ({
    id,
    name: id,
    description: '',
    required_capabilities: [],
    enabled: true,
    last_seen: 0,
    status: 'online',
    metadata: {},
    ...over,
  }) as AgentMetadata;

const faces = () => screen.getByTestId('new-chat-faces');

beforeEach(() => {
  vi.clearAllMocks();
  api.getAgentAccess.mockResolvedValue({ entries: [] });
  agentCtx.agents = [agent('a'), agent('b')];
  agentCtx.refetchAgents.mockResolvedValue(undefined);
  convCtx.conversations = [];
  convCtx.draft = { key: 'draft:1', agentId: 'a' };
});

describe('the new chat', () => {
  it('faces the agent the draft is turned to, and the composer is theirs', () => {
    render(<NewChatScreen />);
    // Who is to either side is named: "create an agent" on the left, b on the right.
    expect(screen.getByLabelText('new_chat.previous').getAttribute('title')).toBe('new_chat.create_agent');
    expect(screen.getByLabelText('new_chat.next').getAttribute('title')).toBe('b');
    expect(screen.getByText('a')).toBeTruthy();
    expect(screen.getByText('composer').getAttribute('data-agent')).toBe('a');
    expect((screen.getByText('composer') as HTMLButtonElement).disabled).toBe(false);
  });

  it('turns by the arrows, and stops at the last agent', () => {
    const { rerender } = render(<NewChatScreen />);
    fireEvent.click(screen.getByLabelText('new_chat.next'));
    expect(convCtx.setDraftAgent).toHaveBeenLastCalledWith('b');

    convCtx.draft = { key: 'draft:1', agentId: 'b' };
    rerender(<NewChatScreen />);
    // Past the last agent there is nobody: no arrow, no face peeking.
    expect(screen.queryByLabelText('new_chat.next')).toBeNull();
    expect(screen.getByLabelText('new_chat.previous').getAttribute('title')).toBe('a');
  });

  it('reaches "create an agent" at the left end, one turn from the first agent, where nothing can be sent', () => {
    const { rerender } = render(<NewChatScreen />);
    fireEvent.click(screen.getByLabelText('new_chat.previous'));
    expect(convCtx.setDraftAgent).toHaveBeenLastCalledWith(null);

    convCtx.draft = { key: 'draft:1', agentId: null };
    rerender(<NewChatScreen />);
    expect(screen.getByText('new_chat.create_agent')).toBeTruthy();
    expect((screen.getByText('composer') as HTMLButtonElement).disabled).toBe(true);
    // It is the end: nothing is further left, and the first agent is to its right.
    expect(screen.queryByLabelText('new_chat.previous')).toBeNull();
    expect(screen.getByLabelText('new_chat.next').getAttribute('title')).toBe('a');
    fireEvent.click(screen.getByLabelText('new_chat.next'));
    expect(convCtx.setDraftAgent).toHaveBeenLastCalledWith('a');
  });

  it('turns by a drag that goes far enough, and not by one that does not', () => {
    render(<NewChatScreen />);
    const el = faces();
    el.setPointerCapture = vi.fn();
    fireEvent.pointerDown(el, { button: 0, clientX: 300, pointerId: 1 });
    fireEvent.pointerUp(el, { clientX: 270, pointerId: 1 });
    expect(convCtx.setDraftAgent).not.toHaveBeenCalled();

    fireEvent.pointerDown(el, { button: 0, clientX: 300, pointerId: 2 });
    fireEvent.pointerMove(el, { clientX: 200, pointerId: 2 });
    fireEvent.pointerUp(el, { clientX: 200, pointerId: 2 });
    expect(convCtx.setDraftAgent).toHaveBeenCalledTimes(1);
    expect(convCtx.setDraftAgent).toHaveBeenLastCalledWith('b');
  });

  it('turns once per swipe, however many wheel events the gesture goes on emitting', () => {
    render(<NewChatScreen />);
    const el = faces();
    for (let i = 0; i < 12; i++) fireEvent.wheel(el, { deltaX: 20, deltaY: 0 });
    expect(convCtx.setDraftAgent).toHaveBeenCalledTimes(1);
    expect(convCtx.setDraftAgent).toHaveBeenLastCalledWith('b');
  });

  it('leaves a vertical scroll alone', () => {
    render(<NewChatScreen />);
    for (let i = 0; i < 12; i++) fireEvent.wheel(faces(), { deltaX: 5, deltaY: 40 });
    expect(convCtx.setDraftAgent).not.toHaveBeenCalled();
  });

  it('shows only "create an agent" when nobody exists', () => {
    agentCtx.agents = [];
    convCtx.draft = { key: 'draft:1', agentId: null };
    render(<NewChatScreen />);
    expect(screen.getByText('new_chat.create_agent')).toBeTruthy();
    expect(screen.getByText('new_chat.create_first')).toBeTruthy();
    expect(screen.queryByLabelText('new_chat.previous')).toBeNull();
    expect(screen.queryByLabelText('new_chat.next')).toBeNull();
  });

  it('cannot send to an agent that is off', () => {
    agentCtx.agents = [agent('a', { enabled: false })];
    render(<NewChatScreen />);
    expect((screen.getByText('composer') as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText('roster.state_stopped')).toBeTruthy();
  });

  it('hands the first message to the draft, creating nothing itself', () => {
    render(<NewChatScreen />);
    fireEvent.click(screen.getByText('composer'));
    expect(convCtx.sendDraft).toHaveBeenCalledWith({ blocks: [], rawText: 'hi', engineOverride: null });
  });

  it('turns to the agent that was just made, once the list has them', async () => {
    convCtx.draft = { key: 'draft:1', agentId: null };
    const { rerender } = render(<NewChatScreen />);
    fireEvent.click(screen.getByText('new_chat.create_agent'));
    await act(async () => {
      fireEvent.click(screen.getByText('create-modal'));
    });
    expect(agentCtx.refetchAgents).toHaveBeenCalled();
    expect(convCtx.setDraftAgent).not.toHaveBeenCalled();

    agentCtx.agents = [agent('a'), agent('b'), agent('agent.newcomer', { name: 'Newcomer' })];
    rerender(<NewChatScreen />);
    expect(convCtx.setDraftAgent).toHaveBeenLastCalledWith('agent.newcomer');
  });

  it('says it when the new agent exists but their face could not be saved, and still turns to them', async () => {
    created.faceProblem = 'disk full';
    convCtx.draft = { key: 'draft:1', agentId: null };
    const { rerender } = render(<NewChatScreen />);
    fireEvent.click(screen.getByText('new_chat.create_agent'));
    await act(async () => {
      fireEvent.click(screen.getByText('create-modal'));
    });
    expect(screen.getByRole('alert').textContent).toBe('create.face_not_saved');
    agentCtx.agents = [agent('a'), agent('agent.newcomer', { name: 'Newcomer' })];
    rerender(<NewChatScreen />);
    expect(convCtx.setDraftAgent).toHaveBeenLastCalledWith('agent.newcomer');
    created.faceProblem = null;
  });

  it('says nothing about a face when the creation had no trouble with one', async () => {
    convCtx.draft = { key: 'draft:1', agentId: null };
    render(<NewChatScreen />);
    fireEvent.click(screen.getByText('new_chat.create_agent'));
    await act(async () => {
      fireEvent.click(screen.getByText('create-modal'));
    });
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('makes the empty composer the way in to creating an agent, and only while nobody is facing', () => {
    const { rerender } = render(<NewChatScreen />);
    expect(screen.queryByText(/^invitation:/)).toBeNull();

    convCtx.draft = { key: 'draft:1', agentId: null };
    rerender(<NewChatScreen />);
    expect(screen.queryByText('create-modal')).toBeNull();
    expect(screen.getByText('invitation:new_chat.create_agent').getAttribute('title')).toBe(
      'new_chat.name_placeholder',
    );
    fireEvent.click(screen.getByText('invitation:new_chat.create_agent'));
    expect(screen.getByText(/create-modal/)).toBeTruthy();
    // The name typed into the box is where the form starts.
    expect(screen.getByText('named:Mira')).toBeTruthy();
  });
});
