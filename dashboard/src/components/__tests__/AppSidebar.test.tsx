import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Conversation } from '../../types';

// Echo i18n keys so the assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}));
const navigate = vi.hoisted(() => vi.fn());
vi.mock('react-router-dom', () => ({
  useLocation: () => ({ pathname: '/' }),
  useNavigate: () => navigate,
}));
vi.mock('../../hooks/useApi', () => ({ useApi: () => ({ post: vi.fn() }) }));
vi.mock('../../hooks/useModules', () => ({ useModules: () => ({ modules: [] }) }));
vi.mock('../../lib/tauri', () => ({ isExperimentalBuild: false }));
vi.mock('../NotificationBell', () => ({ NotificationBell: () => null }));
const connection = vi.hoisted(() => ({ connected: true, checking: false }));
vi.mock('../../contexts/ConnectionContext', () => ({ useConnection: () => connection }));
vi.mock('../ShutdownOverlay', () => ({ requestShutdown: vi.fn() }));

const conversations = vi.hoisted(() => ({
  open: vi.fn(),
  startDraft: vi.fn(),
  leaveDraft: vi.fn(),
  draft: null as { key: string; agentId: string | null } | null,
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
  processingAgentIds: new Set<string>(['agent.b']),
}));
vi.mock('../../contexts/ConversationContext', () => ({
  useConversations: () => ({
    conversations: conversations.list,
    openFor: conversations.openFor,
    open: conversations.open,
    startDraft: conversations.startDraft,
    leaveDraft: conversations.leaveDraft,
    get draft() {
      return conversations.draft;
    },
    rename: conversations.rename,
    archive: conversations.archive,
    remove: conversations.remove,
  }),
}));
vi.mock('../../contexts/AgentContext', () => ({ useAgentContext: () => agentCtx }));

import { AppSidebar } from '../AppSidebar';

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

const DAY = 24 * 60 * 60 * 1000;

beforeEach(() => {
  navigate.mockReset();
  conversations.open.mockReset();
  conversations.startDraft.mockReset();
  conversations.leaveDraft.mockReset();
  conversations.draft = null;
  conversations.archive.mockReset();
  conversations.remove.mockReset();
  conversations.openFor.mockReset().mockReturnValue('today-1');
  const now = Date.now();
  conversations.list = [
    conv('today-1', 'agent.a', now - 60_000, 'Plans'),
    conv('yday-1', 'agent.b', now - DAY, ''),
    conv('old-1', 'agent.a', now - 40 * DAY, 'Long ago'),
  ];
});

describe("the sidebar's conversations", () => {
  it('draws the day groups the mock draws, and keeps older threads behind show more', () => {
    render(<AppSidebar onSettingsClick={vi.fn()} />);
    expect(screen.getByText('group_today')).toBeTruthy();
    expect(screen.getByText('group_yesterday')).toBeTruthy();
    expect(screen.queryByText('group_older')).toBeNull();
    expect(screen.queryByTitle('Long ago')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'show_more' }));
    expect(screen.getByText('group_older')).toBeTruthy();
    expect(screen.getByTitle('Long ago')).toBeTruthy();
  });

  it("marks the open thread, colours the present agent's rows, and shows who is responding", () => {
    render(<AppSidebar onSettingsClick={vi.fn()} />);
    const open = screen.getByTitle('Plans');
    expect(open.getAttribute('aria-current')).toBe('true');
    expect(open.className).toContain('on');
    expect(open.className).toContain('mine');
    const other = screen.getByTitle('untitled_conversation');
    expect(other.getAttribute('aria-current')).toBeNull();
    expect(other.className).not.toContain('mine');
    expect(other.querySelector('.live')).not.toBeNull();
    expect(open.querySelector('.live')).toBeNull();
    expect(screen.getByText('KS22')).toBeTruthy();
  });

  it('opens a thread with its own agent, not the selected one', () => {
    render(<AppSidebar onSettingsClick={vi.fn()} />);
    fireEvent.click(screen.getByTitle('untitled_conversation'));
    expect(conversations.open).toHaveBeenCalledWith('agent.b', 'yday-1');
  });

  it('new chat — the button and ⌘N — opens the new chat on the present agent, and creates nothing', () => {
    render(<AppSidebar onSettingsClick={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: 'new_chat' }));
    expect(conversations.startDraft).toHaveBeenCalledTimes(1);
    expect(conversations.startDraft).toHaveBeenLastCalledWith('agent.a');
    fireEvent.keyDown(window, { key: 'n', metaKey: true });
    expect(conversations.startDraft).toHaveBeenCalledTimes(2);
    // Opening a conversation is what clicking a row does, not this.
    expect(conversations.open).not.toHaveBeenCalled();
  });

  it('marks no thread as open while the new chat is, and has no Chat link to go back by', () => {
    conversations.draft = { key: 'draft:1', agentId: 'agent.a' };
    render(<AppSidebar onSettingsClick={vi.fn()} />);
    expect(screen.getByTitle('Plans').getAttribute('aria-current')).toBeNull();
    expect(screen.queryByText('chat')).toBeNull();
  });

  it('archives from the row menu, and deletes only after confirming', async () => {
    render(<AppSidebar onSettingsClick={vi.fn()} />);
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

  it('shows the kernel state in the footer as the mock does', () => {
    render(<AppSidebar onSettingsClick={vi.fn()} />);
    expect(screen.getByText('kernel_running')).toBeTruthy();
    expect(screen.getByText('agents_count')).toBeTruthy();
  });
});

describe('what the window header used to carry', () => {
  it('opens the help from the navigation, and draws no link when there is no help to open', () => {
    const onHelp = vi.fn();
    const { unmount } = render(<AppSidebar onSettingsClick={vi.fn()} onHelpClick={onHelp} />);
    fireEvent.click(screen.getByText('help'));
    expect(onHelp).toHaveBeenCalledTimes(1);
    unmount();

    render(<AppSidebar onSettingsClick={vi.fn()} />);
    expect(screen.queryByText('help')).toBeNull();
  });

  it('says so when the kernel cannot be reached, instead of "running"', () => {
    connection.connected = false;
    try {
      render(<AppSidebar onSettingsClick={vi.fn()} />);
      expect(screen.getByText('kernel_unreachable')).toBeTruthy();
      expect(screen.queryByText('kernel_running')).toBeNull();
    } finally {
      connection.connected = true;
    }
  });

  it('turns the version into the way to the update once one is announced', () => {
    render(<AppSidebar onSettingsClick={vi.fn()} />);
    expect(screen.queryByRole('button', { name: /update_available_banner/ })).toBeNull();

    const opened = vi.fn();
    window.addEventListener('cloto-open-settings', opened);
    act(() => {
      window.dispatchEvent(new CustomEvent('cloto-update-available', { detail: { version: '9.9.9' } }));
    });
    fireEvent.click(screen.getByRole('button', { name: /update_available_banner/ }));
    window.removeEventListener('cloto-open-settings', opened);

    expect(opened).toHaveBeenCalledTimes(1);
    expect((opened.mock.calls[0][0] as CustomEvent).detail).toEqual({ section: 'about' });
  });
});
