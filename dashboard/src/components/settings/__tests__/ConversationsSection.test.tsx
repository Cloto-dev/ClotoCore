import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Conversation } from '../../../types';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string, o?: Record<string, unknown>) => (o ? `${k}:${JSON.stringify(o)}` : k),
  }),
}));
const api = vi.hoisted(() => ({
  listConversations: vi.fn(),
  updateConversation: vi.fn(),
  deleteConversation: vi.fn(),
  archiveAllConversations: vi.fn(),
  deleteAllConversations: vi.fn(),
}));
vi.mock('../../../hooks/useApi', () => ({ useApi: () => api }));
const agentCtx = vi.hoisted(() => ({
  agents: [
    { id: 'agent.a', name: 'Aster' },
    { id: 'agent.b', name: 'Birch' },
  ],
}));
vi.mock('../../../contexts/AgentContext', () => ({ useAgentContext: () => agentCtx }));
const convCtx = vi.hoisted(() => ({ refresh: vi.fn() }));
vi.mock('../../../contexts/ConversationContext', () => ({ useConversations: () => convCtx }));
const identityCtx = vi.hoisted(() => ({ identity: { id: 'user.1', name: 'User' } }));
vi.mock('../../../contexts/UserIdentityContext', () => ({ useUserIdentity: () => identityCtx }));
vi.mock('../../ui/ConfirmDialog', () => ({ ConfirmDialog: () => null }));

import { ConversationsSection } from '../ConversationsSection';

const archived = (id: string, agentId: string, title: string): Conversation =>
  ({
    id,
    agent_id: agentId,
    title,
    archived_at: 1_700_000_000,
    created_at: 1,
    updated_at: 1,
  }) as unknown as Conversation;

const EMPTY = 'conversations.archived_empty';

beforeEach(() => {
  for (const f of Object.values(api)) f.mockReset();
});

describe('archived conversations', () => {
  it('says nothing is archived only when every agent was read and none has any', async () => {
    api.listConversations.mockResolvedValue([]);
    render(<ConversationsSection />);
    await screen.findByText(EMPTY);
    expect(api.listConversations).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('does not say nothing is archived before anything has been read', () => {
    api.listConversations.mockReturnValue(new Promise(() => {}));
    render(<ConversationsSection />);
    expect(screen.queryByText(EMPTY)).toBeNull();
  });

  it('does not call a list it could not read empty: it names the agent and offers another try', async () => {
    api.listConversations.mockImplementation((agentId: string) =>
      agentId === 'agent.b' ? Promise.reject(new Error('429')) : Promise.resolve([]),
    );
    render(<ConversationsSection />);
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('"agents":"Birch"');
    expect(alert.textContent).not.toContain('Aster');
    // The kernel may well hold archived conversations for Birch.
    expect(screen.queryByText(EMPTY)).toBeNull();

    api.listConversations.mockReset();
    api.listConversations.mockImplementation((agentId: string) =>
      Promise.resolve(agentId === 'agent.b' ? [archived('c1', 'agent.b', 'kept whole')] : []),
    );
    fireEvent.click(screen.getByRole('button', { name: 'conversations.retry' }));
    await screen.findByText('kept whole');
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  });

  it('still lists what it could read beside the warning', async () => {
    api.listConversations.mockImplementation((agentId: string) =>
      agentId === 'agent.b'
        ? Promise.reject(new Error('429'))
        : Promise.resolve([archived('c2', 'agent.a', 'readable')]),
    );
    render(<ConversationsSection />);
    await screen.findByText('readable');
    expect(screen.getByRole('alert')).toBeTruthy();
  });
});
