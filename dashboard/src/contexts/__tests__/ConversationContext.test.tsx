import { act, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Conversation } from '../../types';

/**
 * Which conversation the console opens (docs/CONVERSATIONS_DESIGN.md §2b):
 * the remembered one if it still exists, else the newest, else a new one —
 * and only one new one however many mounts ask at once.
 */
const api = vi.hoisted(() => ({
  listConversations: vi.fn(),
  createConversation: vi.fn(),
  updateConversation: vi.fn(),
  deleteConversation: vi.fn(),
}));
const navigate = vi.hoisted(() => vi.fn());
const agentCtx = vi.hoisted(() => ({
  agents: [
    { id: 'agent.a', name: 'A' },
    { id: 'agent.b', name: 'B' },
  ],
  setSelectedAgentId: vi.fn(),
  setSystemActive: vi.fn(),
}));

vi.mock('../../hooks/useApi', () => ({ useApi: () => api }));
vi.mock('react-router-dom', () => ({ useNavigate: () => navigate }));
vi.mock('../AgentContext', () => ({ useAgentContext: () => agentCtx }));
vi.mock('../UserIdentityContext', () => ({ useUserIdentity: () => ({ identity: { id: 'u1', name: 'User' } }) }));

import { ConversationProvider, useConversations } from '../ConversationContext';

function conv(id: string, agent_id: string, updated_at: number): Conversation {
  return {
    id,
    agent_id,
    user_id: 'u1',
    title: id,
    created_at: updated_at,
    updated_at,
    archived_at: null,
    message_count: 1,
  };
}

type Ctx = ReturnType<typeof useConversations>;
let ctx: Ctx | null = null;
function Probe() {
  ctx = useConversations();
  return null;
}
function mount() {
  render(
    <ConversationProvider>
      <Probe />
    </ConversationProvider>,
  );
}

beforeEach(() => {
  localStorage.clear();
  ctx = null;
  api.listConversations.mockReset();
  api.createConversation.mockReset();
  api.updateConversation.mockReset();
  api.deleteConversation.mockReset();
  navigate.mockReset();
  let n = 0;
  api.createConversation.mockImplementation(async (agentId: string) => conv(`new-${++n}`, agentId, Date.now()));
});
afterEach(() => localStorage.clear());

describe('choosing the open conversation', () => {
  it('prefers the remembered conversation while it still exists', async () => {
    api.listConversations.mockResolvedValue([conv('newest', 'agent.a', 3000), conv('older', 'agent.a', 1000)]);
    localStorage.setItem('cloto-open-conversation:agent.a', 'older');
    mount();
    await waitFor(() => expect(ctx).not.toBeNull());
    let id: string | null = '';
    await act(async () => {
      id = await (ctx as Ctx).resolveOpen('agent.a');
    });
    expect(id).toBe('older');
    expect(api.createConversation).not.toHaveBeenCalled();
  });

  it('falls back to the newest when the remembered one is gone', async () => {
    api.listConversations.mockResolvedValue([conv('newest', 'agent.a', 3000), conv('older', 'agent.a', 1000)]);
    localStorage.setItem('cloto-open-conversation:agent.a', 'deleted-elsewhere');
    mount();
    await waitFor(() => expect(ctx).not.toBeNull());
    let id: string | null = '';
    await act(async () => {
      id = await (ctx as Ctx).resolveOpen('agent.a');
    });
    expect(id).toBe('newest');
    expect(localStorage.getItem('cloto-open-conversation:agent.a')).toBe('newest');
  });

  it('opens the new chat, and creates nothing, when nobody has spoken with the agent yet', async () => {
    api.listConversations.mockResolvedValue([]);
    mount();
    await waitFor(() => expect(ctx).not.toBeNull());
    let id: string | null = 'unset';
    await act(async () => {
      id = await (ctx as Ctx).resolveOpen('agent.a');
    });
    expect(id).toBeNull();
    expect((ctx as Ctx).draft?.agentId).toBe('agent.a');
    expect(api.createConversation).not.toHaveBeenCalled();
  });
});

describe('the new chat', () => {
  it('creates nothing and lists nothing until the first message is sent', async () => {
    api.listConversations.mockResolvedValue([conv('old', 'agent.a', 1000)]);
    mount();
    await waitFor(() => expect((ctx as Ctx).conversations.length).toBe(2));
    act(() => (ctx as Ctx).startDraft('agent.a'));
    expect((ctx as Ctx).draft).toMatchObject({ agentId: 'agent.a' });
    expect(api.createConversation).not.toHaveBeenCalled();
    expect((ctx as Ctx).conversations.length).toBe(2);
    expect(agentCtx.setSelectedAgentId).toHaveBeenLastCalledWith('agent.a');
    expect(navigate).toHaveBeenLastCalledWith('/?agent=agent.a', { replace: false });

    // Leaving without a word leaves nothing behind.
    act(() => (ctx as Ctx).leaveDraft());
    expect((ctx as Ctx).draft).toBeNull();
    expect(api.createConversation).not.toHaveBeenCalled();
  });

  it('turns to another agent, or to nobody, as the same draft', async () => {
    api.listConversations.mockResolvedValue([]);
    mount();
    await waitFor(() => expect(ctx).not.toBeNull());
    act(() => (ctx as Ctx).startDraft('agent.a'));
    const key = (ctx as Ctx).draft?.key;
    act(() => (ctx as Ctx).setDraftAgent('agent.b'));
    expect((ctx as Ctx).draft).toEqual({ key, agentId: 'agent.b' });
    expect(navigate).toHaveBeenLastCalledWith('/?agent=agent.b', { replace: true });
    act(() => (ctx as Ctx).setDraftAgent(null));
    expect((ctx as Ctx).draft).toEqual({ key, agentId: null });
    expect(agentCtx.setSelectedAgentId).toHaveBeenLastCalledWith(null);
    // Pressing New chat again is asking for an empty one: a new key.
    act(() => (ctx as Ctx).startDraft('agent.a'));
    expect((ctx as Ctx).draft?.key).not.toBe(key);
  });

  it('cannot be sent while it faces nobody', async () => {
    api.listConversations.mockResolvedValue([]);
    mount();
    await waitFor(() => expect(ctx).not.toBeNull());
    act(() => (ctx as Ctx).startDraft(null));
    act(() => (ctx as Ctx).sendDraft({ blocks: [], rawText: 'hi', engineOverride: null }));
    expect((ctx as Ctx).draft?.first).toBeUndefined();
  });

  it('becomes a conversation with its first message: created once, listed first, opened, mounted under the same key', async () => {
    api.listConversations.mockResolvedValue([conv('old', 'agent.a', 1000)]);
    mount();
    await waitFor(() => expect((ctx as Ctx).conversations.length).toBe(2));
    act(() => (ctx as Ctx).startDraft('agent.a'));
    const key = (ctx as Ctx).draft?.key as string;
    act(() => (ctx as Ctx).sendDraft({ blocks: [], rawText: 'hi', engineOverride: null }));
    expect((ctx as Ctx).draft?.first?.rawText).toBe('hi');

    let ids: string[] = [];
    await act(async () => {
      // A double send must not mint two.
      ids = await Promise.all([(ctx as Ctx).commitDraft('agent.a'), (ctx as Ctx).commitDraft('agent.a')]);
    });
    expect(api.createConversation).toHaveBeenCalledTimes(1);
    expect(api.createConversation).toHaveBeenCalledWith('agent.a', 'u1');
    expect(ids).toEqual(['new-1', 'new-1']);
    expect((ctx as Ctx).conversations[0].id).toBe('new-1');
    expect((ctx as Ctx).openFor('agent.a')).toBe('new-1');
    expect((ctx as Ctx).draft).toBeNull();
    // The console is not remounted in the middle of its first exchange.
    expect((ctx as Ctx).mountKeyFor('agent.a', 'new-1')).toBe(key);
    expect((ctx as Ctx).mountKeyFor('agent.a', 'old')).toBe('agent.a:old');
  });

  it('is left when a conversation is chosen', async () => {
    api.listConversations.mockResolvedValue([conv('old', 'agent.a', 1000)]);
    mount();
    await waitFor(() => expect(ctx).not.toBeNull());
    act(() => (ctx as Ctx).startDraft('agent.a'));
    act(() => (ctx as Ctx).open('agent.a', 'old'));
    expect((ctx as Ctx).draft).toBeNull();
    expect((ctx as Ctx).openFor('agent.a')).toBe('old');
  });
});

describe('acting on a conversation', () => {
  it('delete forgets the remembered choice; archive re-reads the list', async () => {
    api.listConversations.mockResolvedValue([conv('c1', 'agent.a', 1000)]);
    api.deleteConversation.mockResolvedValue({ deleted_messages: 1 });
    api.updateConversation.mockResolvedValue(conv('c1', 'agent.a', 1000));
    mount();
    await waitFor(() => expect(ctx).not.toBeNull());
    await act(async () => {
      await (ctx as Ctx).resolveOpen('agent.a');
    });
    expect((ctx as Ctx).openFor('agent.a')).toBe('c1');

    const listCallsBefore = api.listConversations.mock.calls.length;
    await act(async () => {
      await (ctx as Ctx).archive('agent.a', 'c1', true);
    });
    expect(api.updateConversation).toHaveBeenCalledWith('agent.a', 'c1', { archived: true });
    expect(api.listConversations.mock.calls.length).toBeGreaterThan(listCallsBefore);

    await act(async () => {
      await (ctx as Ctx).remove('agent.a', 'c1');
    });
    expect(api.deleteConversation).toHaveBeenCalledWith('agent.a', 'c1');
    expect((ctx as Ctx).openFor('agent.a')).toBeNull();
    expect(localStorage.getItem('cloto-open-conversation:agent.a')).toBeNull();
  });
});
