import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata, Conversation, McpServerInfo, Memory } from '../../types';

const i18n = vi.hoisted(() => ({
  t: (k: string, o?: Record<string, unknown>) => (o ? `${k}:${Object.values(o).join('|')}` : k),
}));
vi.mock('react-i18next', () => ({ useTranslation: () => i18n }));
const navigate = vi.hoisted(() => vi.fn());
vi.mock('react-router-dom', () => ({ useNavigate: () => navigate }));
const agentCtx = vi.hoisted(() => ({
  agents: [] as AgentMetadata[],
  setSelectedAgentId: vi.fn(),
  setSystemActive: vi.fn(),
}));
vi.mock('../../contexts/AgentContext', () => ({ useAgentContext: () => agentCtx }));
const convCtx = vi.hoisted(() => ({ conversations: [] as Conversation[], open: vi.fn(), leaveDraft: vi.fn() }));
vi.mock('../../contexts/ConversationContext', () => ({ useConversations: () => convCtx }));
const api = vi.hoisted(() => ({ getMemories: vi.fn() }));
vi.mock('../../hooks/useApi', () => ({ useApi: () => api }));
const data = vi.hoisted(() => ({ servers: [] as McpServerInfo[] }));
vi.mock('../../hooks/useMcpServers', () => ({ useMcpServers: () => data }));

import { CommandPalette } from '../CommandPalette';

const conv = (id: string, agentId: string, title: string) =>
  ({ id, agent_id: agentId, title, archived_at: null }) as unknown as Conversation;
const memory = (id: number, agentId: string, content: string) =>
  ({ id, agent_id: agentId, content }) as unknown as Memory;

beforeEach(() => {
  vi.clearAllMocks();
  agentCtx.agents = [
    { id: 'agent.sapphy', name: 'Sapphy' },
    { id: 'agent.karin', name: 'Karin' },
  ] as AgentMetadata[];
  convCtx.conversations = [
    conv('c1', 'agent.sapphy', 'Recall precision notes'),
    conv('c2', 'agent.karin', 'Morning greeting'),
    conv('c3', 'agent.karin', ''),
  ];
  data.servers = [
    { id: 'mind.deepseek', status: 'Connected', tools: [], description: 'Reasoning over the API' },
    { id: 'memory.cpersona', status: 'Connected', tools: [], description: 'Long-term memory' },
  ] as unknown as McpServerInfo[];
  api.getMemories.mockResolvedValue({
    memories: [
      memory(1, 'agent.sapphy', 'the recall gate is the thing to move'),
      memory(2, 'agent.karin', 'tea at four'),
    ],
    capabilities: {},
  });
});

const input = () => screen.getByRole('combobox');
const options = () => screen.queryAllByRole('option');
const labels = () => options().map((o) => o.querySelector('.l')?.textContent);
async function draw(onClose = vi.fn()) {
  const view = render(<CommandPalette onClose={onClose} />);
  await waitFor(() => expect(api.getMemories).toHaveBeenCalled());
  return { ...view, onClose };
}

describe('what it offers', () => {
  it('lists the screens and the conversations before anything is typed, and no memories or servers', async () => {
    await draw();
    expect(screen.getByTestId('palette-screens')).toBeTruthy();
    expect(within(screen.getByTestId('palette-conversations')).getAllByRole('option')).toHaveLength(3);
    expect(screen.queryByTestId('palette-servers')).toBeNull();
    expect(screen.queryByTestId('palette-memories')).toBeNull();
  });

  it('finds a conversation by its title and by who it is with, whatever the case', async () => {
    await draw();
    fireEvent.change(input(), { target: { value: 'RECALL' } });
    expect(within(screen.getByTestId('palette-conversations')).getAllByRole('option')).toHaveLength(1);
    fireEvent.change(input(), { target: { value: 'karin' } });
    const found = within(screen.getByTestId('palette-conversations')).getAllByRole('option');
    // The untitled one is found by its agent, under the untitled wording.
    expect(found.map((o) => o.querySelector('.l')?.textContent)).toEqual(['Morning greeting', 'untitled_conversation']);
  });

  it('finds a server by its description and a memory by what it says', async () => {
    await draw();
    fireEvent.change(input(), { target: { value: 'memory' } });
    expect(within(screen.getByTestId('palette-servers')).getAllByRole('option')[0].textContent).toContain('cpersona');
    fireEvent.change(input(), { target: { value: 'gate' } });
    await waitFor(() => expect(screen.getByTestId('palette-memories')).toBeTruthy());
    expect(within(screen.getByTestId('palette-memories')).getAllByRole('option')[0].textContent).toContain(
      'the recall gate',
    );
  });

  it('says it could not read the memories rather than finding none', async () => {
    api.getMemories.mockRejectedValue(new Error('down'));
    await draw();
    fireEvent.change(input(), { target: { value: 'gate' } });
    expect(await screen.findByText('palette.memories_failed')).toBeTruthy();
    expect(screen.queryByText('palette.no_match')).toBeNull();
  });

  it('shows at most five of a group and says how many more there are', async () => {
    convCtx.conversations = Array.from({ length: 8 }, (_, i) => conv(`c${i}`, 'agent.sapphy', `thread ${i}`));
    await draw();
    fireEvent.change(input(), { target: { value: 'thread' } });
    const group = screen.getByTestId('palette-conversations');
    expect(within(group).getAllByRole('option')).toHaveLength(5);
    expect(group.textContent).toContain('palette.more:3');
  });

  it('says nothing matches when nothing does', async () => {
    await draw();
    fireEvent.change(input(), { target: { value: 'zzzz' } });
    expect(options()).toHaveLength(0);
    expect(screen.getByText('palette.no_match')).toBeTruthy();
  });
});

describe('choosing', () => {
  it('opens a conversation with Enter after moving down to it, and closes first', async () => {
    const { onClose } = await draw();
    fireEvent.change(input(), { target: { value: 'morning' } });
    expect(labels()).toEqual(['Morning greeting']);
    fireEvent.keyDown(input(), { key: 'Enter' });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(convCtx.open).toHaveBeenCalledWith('agent.karin', 'c2');
    expect(onClose.mock.invocationCallOrder[0]).toBeLessThan(convCtx.open.mock.invocationCallOrder[0]);
  });

  it('does not choose on the Enter that confirms a word being converted by an input method', async () => {
    const { onClose } = await draw();
    fireEvent.change(input(), { target: { value: 'morning' } });
    fireEvent.keyDown(input(), { key: 'Enter', isComposing: true });
    expect(onClose).not.toHaveBeenCalled();
    expect(convCtx.open).not.toHaveBeenCalled();
    fireEvent.keyDown(input(), { key: 'Enter' });
    expect(convCtx.open).toHaveBeenCalledWith('agent.karin', 'c2');
  });

  it('moves the active row with the arrows, wrapping at both ends', async () => {
    await draw();
    fireEvent.change(input(), { target: { value: 'settings' } });
    const count = options().length;
    expect(count).toBeGreaterThan(1);
    const activeIndex = () => options().findIndex((o) => o.getAttribute('aria-selected') === 'true');
    expect(activeIndex()).toBe(0);
    fireEvent.keyDown(input(), { key: 'ArrowUp' });
    expect(activeIndex()).toBe(count - 1);
    fireEvent.keyDown(input(), { key: 'ArrowDown' });
    expect(activeIndex()).toBe(0);
    fireEvent.keyDown(input(), { key: 'ArrowDown' });
    expect(activeIndex()).toBe(1);
    expect(input().getAttribute('aria-activedescendant')).toBe(options()[1].id);
  });

  it('goes to a screen the way the sidebar does, leaving the chat behind', async () => {
    await draw();
    fireEvent.change(input(), { target: { value: 'cron' } });
    fireEvent.click(options()[0]);
    expect(convCtx.leaveDraft).toHaveBeenCalled();
    expect(agentCtx.setSelectedAgentId).toHaveBeenCalledWith(null);
    expect(navigate).toHaveBeenCalledWith('/cron');
  });

  it('links a server to its page and a memory to the memory screen with what was typed', async () => {
    await draw();
    fireEvent.change(input(), { target: { value: 'reasoning' } });
    fireEvent.click(within(screen.getByTestId('palette-servers')).getAllByRole('option')[0]);
    expect(navigate).toHaveBeenLastCalledWith('/mcp-servers?server=mind.deepseek');

    fireEvent.change(input(), { target: { value: ' tea at ' } });
    await waitFor(() => expect(screen.getByTestId('palette-memories')).toBeTruthy());
    fireEvent.click(within(screen.getByTestId('palette-memories')).getAllByRole('option')[0]);
    expect(navigate).toHaveBeenLastCalledWith('/dashboard?q=tea%20at');
  });

  it('goes to a settings section by its name', async () => {
    await draw();
    fireEvent.change(input(), { target: { value: 'sections.about' } });
    fireEvent.keyDown(input(), { key: 'Enter' });
    expect(navigate).toHaveBeenCalledWith('/settings?section=about');
  });
});

describe('leaving', () => {
  it('closes on Escape without letting the key reach anything behind it', async () => {
    const behind = vi.fn();
    document.addEventListener('keydown', behind);
    const { onClose } = await draw();
    fireEvent.keyDown(input(), { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(behind).not.toHaveBeenCalled();
    document.removeEventListener('keydown', behind);
  });

  it('keeps Tab in the field, and gives the focus back to what had it when it goes', async () => {
    const button = document.createElement('button');
    document.body.appendChild(button);
    button.focus();
    const { unmount } = await draw();
    expect(document.activeElement).toBe(input());
    const tab = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    act(() => {
      input().dispatchEvent(tab);
    });
    expect(tab.defaultPrevented).toBe(true);
    unmount();
    expect(document.activeElement).toBe(button);
    button.remove();
  });

  it('closes on a click on the backdrop and not on a click inside', async () => {
    const { onClose, container } = await draw();
    fireEvent.mouseDown(screen.getByRole('dialog'));
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.mouseDown(container.querySelector('.palette-backdrop') as Element);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
