import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Conversation } from '../../types';

// Echo i18n keys so the assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}));
vi.mock('react-router-dom', () => ({ useLocation: () => ({ pathname: '/' }) }));

const conversations = vi.hoisted(() => ({
  open: vi.fn(),
  newChat: vi.fn(),
  rename: vi.fn(),
  archive: vi.fn(),
  remove: vi.fn(),
  openFor: vi.fn(),
  list: [] as unknown[],
}));
const agentCtx = vi.hoisted(() => ({
  agents: [
    { id: 'agent.a', name: 'Sapphy' },
    { id: 'agent.b', name: 'KS22' },
  ],
  selectedAgentId: 'agent.a' as string | null,
  systemActive: false,
  setSelectedAgentId: vi.fn(),
  setSystemActive: vi.fn(),
}));
vi.mock('../../contexts/ConversationContext', () => ({
  useConversations: () => ({
    conversations: conversations.list,
    openFor: conversations.openFor,
    open: conversations.open,
    newChat: conversations.newChat,
    rename: conversations.rename,
    archive: conversations.archive,
    remove: conversations.remove,
  }),
}));
vi.mock('../../contexts/AgentContext', () => ({ useAgentContext: () => agentCtx }));

import { ConversationList } from '../ConversationList';

function conv(id: string, agent_id: string, updated_at: number, title = id): Conversation {
  return {
    id,
    agent_id,
    user_id: 'u1',
    title,
    created_at: updated_at,
    updated_at,
    archived_at: null,
    message_count: 1,
  };
}

beforeEach(() => {
  conversations.open.mockReset();
  conversations.newChat.mockReset();
  conversations.archive.mockReset();
  conversations.remove.mockReset();
  conversations.openFor.mockReset().mockReturnValue('today-1');
  const now = Date.now();
  conversations.list = [
    conv('today-1', 'agent.a', now - 60_000, 'Plans'),
    conv('old-1', 'agent.b', now - 40 * 24 * 60 * 60 * 1000, ''),
  ];
  conversations.newChat.mockResolvedValue(conv('fresh', 'agent.a', now));
});

describe('the conversation list', () => {
  it('shows each thread under its day with the agent it is with, and the open one marked', () => {
    render(<ConversationList collapsed={false} />);
    expect(screen.getByText('group_today')).toBeTruthy();
    expect(screen.getByText('group_older')).toBeTruthy();
    expect(screen.getByText('Plans')).toBeTruthy();
    expect(screen.getByText('Sapphy')).toBeTruthy();
    // An untitled thread reads as a new chat, and the other agent's name is on its row.
    expect(screen.getByText('untitled_conversation')).toBeTruthy();
    expect(screen.getByText('KS22')).toBeTruthy();
    const open = screen.getByTitle('Plans');
    expect(open.getAttribute('aria-current')).toBe('true');
    expect(screen.getByTitle('untitled_conversation').getAttribute('aria-current')).toBeNull();
  });

  it('opens a thread with its own agent, not the selected one', () => {
    render(<ConversationList collapsed={false} />);
    fireEvent.click(screen.getByTitle('untitled_conversation'));
    expect(conversations.open).toHaveBeenCalledWith('agent.b', 'old-1');
  });

  it('new chat creates for the selected agent and opens it', async () => {
    render(<ConversationList collapsed={false} />);
    fireEvent.click(screen.getByRole('button', { name: 'new_chat' }));
    await vi.waitFor(() => expect(conversations.open).toHaveBeenCalledWith('agent.a', 'fresh'));
    expect(conversations.newChat).toHaveBeenCalledWith('agent.a');
  });

  it('archives from the row menu, and deletes only after confirming', async () => {
    render(<ConversationList collapsed={false} />);
    const menus = screen.getAllByRole('button', { name: 'conversation_menu' });
    fireEvent.click(menus[0]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'archive' }));
    await vi.waitFor(() => expect(conversations.archive).toHaveBeenCalledWith('agent.a', 'today-1', true));

    fireEvent.click(menus[0]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'delete' }));
    expect(conversations.remove).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'delete' }));
    await vi.waitFor(() => expect(conversations.remove).toHaveBeenCalledWith('agent.a', 'today-1'));
  });

  it('shows only new chat when collapsed', () => {
    render(<ConversationList collapsed={true} />);
    expect(screen.getByRole('button', { name: 'new_chat' })).toBeTruthy();
    expect(screen.queryByTestId('conversation-list')).toBeNull();
  });
});
