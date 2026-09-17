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
    let id = '';
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
    let id = '';
    await act(async () => {
      id = await (ctx as Ctx).resolveOpen('agent.a');
    });
    expect(id).toBe('newest');
    expect(localStorage.getItem('cloto-open-conversation:agent.a')).toBe('newest');
  });

  it('creates exactly one conversation when there is none, even if asked twice at once', async () => {
    api.listConversations.mockResolvedValue([]);
    mount();
    await waitFor(() => expect(ctx).not.toBeNull());
    let ids: string[] = [];
    await act(async () => {
      ids = await Promise.all([(ctx as Ctx).resolveOpen('agent.a'), (ctx as Ctx).resolveOpen('agent.a')]);
    });
    expect(api.createConversation).toHaveBeenCalledTimes(1);
    expect(api.createConversation).toHaveBeenCalledWith('agent.a', 'u1');
    expect(ids[0]).toBe(ids[1]);
    expect(localStorage.getItem('cloto-open-conversation:agent.a')).toBe(ids[0]);
  });
});

describe('acting on a conversation', () => {
  it('new chat creates for the agent, opens it, and lists it first', async () => {
    api.listConversations.mockResolvedValue([conv('old', 'agent.a', 1000)]);
    mount();
    await waitFor(() => expect((ctx as Ctx).conversations.length).toBe(2));
    await act(async () => {
      const created = await (ctx as Ctx).newChat('agent.a');
      (ctx as Ctx).open('agent.a', created.id);
    });
    expect(api.createConversation).toHaveBeenCalledWith('agent.a', 'u1');
    expect((ctx as Ctx).conversations[0].id).toBe('new-1');
    expect((ctx as Ctx).openFor('agent.a')).toBe('new-1');
    expect(agentCtx.setSelectedAgentId).toHaveBeenCalledWith('agent.a');
    expect(navigate).toHaveBeenCalledWith('/?agent=agent.a');
  });

  it('new chat reuses an empty conversation instead of minting a second one', async () => {
    api.listConversations.mockResolvedValue([{ ...conv('blank', 'agent.a', 5000), message_count: 0 }]);
    mount();
    await waitFor(() => expect((ctx as Ctx).conversations.length).toBe(2));
    let opened = '';
    await act(async () => {
      opened = (await (ctx as Ctx).newChat('agent.a')).id;
    });
    expect(opened).toBe('blank');
    expect(api.createConversation).not.toHaveBeenCalled();
    expect((ctx as Ctx).openFor('agent.a')).toBe('blank');
  });

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
