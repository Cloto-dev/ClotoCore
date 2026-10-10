import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata, Episode, Memory } from '../types';
import { PartnerMemories } from './PartnerMemories';

const m = vi.hoisted(() => ({
  getMemories: vi.fn(),
  getEpisodes: vi.fn(),
  deleteMemory: vi.fn(),
  updateMemory: vi.fn(),
  lockMemory: vi.fn(),
  unlockMemory: vi.fn(),
  deleteEpisode: vi.fn(),
  events: [] as Array<(data: { type: string }) => void>,
}));
vi.mock('../hooks/useApi', () => ({ useApi: () => m }));
vi.mock('../hooks/useEventStream', () => ({
  useEventStream: (_url: string, handler: (data: { type: string }) => void) => {
    m.events.push(handler);
  },
}));
vi.mock('react-i18next', () => {
  const t = (key: string, values?: Record<string, string>) => (values ? `${key} ${JSON.stringify(values)}` : key);
  return { useTranslation: () => ({ t, i18n: { language: 'en' } }) };
});

const agent = { id: 'agent.mio', name: 'Mio', metadata: {} } as AgentMetadata;
const iso = (d: Date) => d.toISOString();
const today = new Date();
const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1, 12);
function memory(id: number, content: string, at: Date, locked = false): Memory {
  return { id, agent_id: agent.id, content, source: {}, timestamp: iso(at), created_at: iso(at), locked };
}
const walk = memory(1, 'A short walk is the right break.', today);
const quiet = memory(2, 'Do not interrupt while focusing.', yesterday);
const name = memory(3, 'Prefers to be called by first name.', yesterday, true);
const episode: Episode = {
  id: 9,
  agent_id: agent.id,
  summary: 'Planned the weekend trip.',
  keywords: 'trip weekend',
  end_time: iso(yesterday),
  created_at: iso(yesterday),
};
const caps = (update: boolean) => ({
  update_memory: update,
  lock_memory: true,
  unlock_memory: true,
  set_recall_precision: false,
  get_recall_precision: false,
});
function show(update = true) {
  m.getMemories.mockResolvedValue({ memories: [walk, quiet, name], capabilities: caps(update) });
  m.getEpisodes.mockResolvedValue([episode]);
  const props = { onBack: vi.fn(), onTalk: vi.fn(), onManage: vi.fn() };
  render(<PartnerMemories agent={agent} {...props} />);
  return props;
}
const item = (text: string) => screen.getByText(text).closest('li') as HTMLElement;

beforeEach(() => {
  vi.resetAllMocks();
  m.events.length = 0;
});

describe('partner memories', () => {
  it("reads only this partner's memories and groups them under their local day", async () => {
    show();
    await screen.findByText(walk.content);
    expect(m.getMemories).toHaveBeenCalledWith(agent.id);
    expect(m.getEpisodes).toHaveBeenCalledWith(agent.id);
    const todayGroup = screen.getByRole('heading', { name: 'mate.today' }).parentElement as HTMLElement;
    const yesterdayGroup = screen.getByRole('heading', { name: 'mate.yesterday' }).parentElement as HTMLElement;
    expect(within(todayGroup).getByText(walk.content)).toBeInTheDocument();
    expect(within(todayGroup).queryByText(quiet.content)).toBeNull();
    expect(within(yesterdayGroup).getByText(quiet.content)).toBeInTheDocument();
    expect(screen.getByText(episode.summary)).toBeInTheDocument();
  });

  it('deletes a memory only after the confirmation, and keeping it calls nothing', async () => {
    m.deleteMemory.mockResolvedValue(undefined);
    show();
    await screen.findByText(walk.content);
    fireEvent.click(within(item(walk.content)).getByRole('button', { name: 'mate.memory_delete' }));
    expect(m.deleteMemory).not.toHaveBeenCalled();
    const keep = within(item(walk.content)).getByRole('button', { name: 'mate.memory_keep' });
    expect(keep).toHaveFocus();
    fireEvent.click(keep);
    expect(m.deleteMemory).not.toHaveBeenCalled();
    expect(within(item(walk.content)).queryByRole('button', { name: 'mate.memory_keep' })).toBeNull();

    fireEvent.click(within(item(walk.content)).getByRole('button', { name: 'mate.memory_delete' }));
    fireEvent.click(within(item(walk.content)).getByRole('button', { name: 'mate.memory_delete_confirmed' }));
    await waitFor(() => expect(screen.queryByText(walk.content)).toBeNull());
    expect(m.deleteMemory).toHaveBeenCalledExactlyOnceWith(walk.id);
    expect(screen.getByText(quiet.content)).toBeInTheDocument();
  });

  it('keeps a memory on screen and says so when the delete fails', async () => {
    m.deleteMemory.mockRejectedValue(new Error('server'));
    show();
    await screen.findByText(walk.content);
    fireEvent.click(within(item(walk.content)).getByRole('button', { name: 'mate.memory_delete' }));
    fireEvent.click(within(item(walk.content)).getByRole('button', { name: 'mate.memory_delete_confirmed' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('mate.memory_change_failed');
    expect(screen.getByText(walk.content)).toBeInTheDocument();
  });

  it('offers no correction or deletion for a protected memory, and unprotects through the unlock call', async () => {
    m.unlockMemory.mockResolvedValue({ lock_level: '' });
    show();
    await screen.findByText(name.content);
    const row = within(item(name.content));
    expect(row.getByText(/mate\.memory_meta_protected/)).toBeInTheDocument();
    expect(row.queryByRole('button', { name: 'mate.memory_edit' })).toBeNull();
    expect(row.queryByRole('button', { name: 'mate.memory_delete' })).toBeNull();
    fireEvent.click(row.getByRole('button', { name: 'mate.memory_unprotect' }));
    await waitFor(() => expect(row.getByRole('button', { name: 'mate.memory_delete' })).toBeInTheDocument());
    expect(m.unlockMemory).toHaveBeenCalledExactlyOnceWith(name.id);
    expect(m.lockMemory).not.toHaveBeenCalled();
  });

  it('protects an unprotected memory through the lock call', async () => {
    m.lockMemory.mockResolvedValue({ lock_level: 'kernel' });
    show();
    await screen.findByText(quiet.content);
    fireEvent.click(within(item(quiet.content)).getByRole('button', { name: 'mate.memory_protect' }));
    await waitFor(() =>
      expect(within(item(quiet.content)).queryByRole('button', { name: 'mate.memory_delete' })).toBeNull(),
    );
    expect(m.lockMemory).toHaveBeenCalledExactlyOnceWith(quiet.id);
    expect(m.unlockMemory).not.toHaveBeenCalled();
  });

  it('saves a trimmed correction, and offers none when the memory server cannot update', async () => {
    m.updateMemory.mockResolvedValue(undefined);
    show();
    await screen.findByText(walk.content);
    fireEvent.click(within(item(walk.content)).getByRole('button', { name: 'mate.memory_edit' }));
    const box = screen.getByLabelText('mate.memory_edit_label');
    expect(box).toHaveFocus();
    const save = screen.getByRole('button', { name: 'mate.memory_save' });
    expect(save).toBeDisabled();
    fireEvent.change(box, { target: { value: '   ' } });
    expect(save).toBeDisabled();
    fireEvent.change(box, { target: { value: '  A long walk is better.  ' } });
    fireEvent.click(save);
    await screen.findByText('A long walk is better.');
    expect(m.updateMemory).toHaveBeenCalledExactlyOnceWith(walk.id, 'A long walk is better.');
  });

  it('hides the correction when the memory server has no update tool', async () => {
    show(false);
    await screen.findByText(walk.content);
    expect(within(item(walk.content)).queryByRole('button', { name: 'mate.memory_edit' })).toBeNull();
    expect(within(item(walk.content)).getByRole('button', { name: 'mate.memory_delete' })).toBeInTheDocument();
  });

  it('deletes a conversation summary through the episode call after confirmation', async () => {
    m.deleteEpisode.mockResolvedValue(undefined);
    show();
    await screen.findByText(episode.summary);
    fireEvent.click(within(item(episode.summary)).getByRole('button', { name: 'mate.memory_delete' }));
    expect(m.deleteEpisode).not.toHaveBeenCalled();
    fireEvent.click(within(item(episode.summary)).getByRole('button', { name: 'mate.memory_delete_confirmed' }));
    await waitFor(() => expect(screen.queryByText(episode.summary)).toBeNull());
    expect(m.deleteEpisode).toHaveBeenCalledExactlyOnceWith(episode.id);
    expect(m.deleteMemory).not.toHaveBeenCalled();
  });

  it('filters memories and summaries by what is typed', async () => {
    show();
    await screen.findByText(walk.content);
    fireEvent.change(screen.getByLabelText('mate.memories_search'), { target: { value: 'WEEKEND' } });
    expect(screen.queryByText(walk.content)).toBeNull();
    expect(screen.getByText(episode.summary)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('mate.memories_search'), { target: { value: 'nothing like this' } });
    expect(screen.getByRole('status')).toHaveTextContent('mate.memories_no_match');
  });

  it('offers to talk when there is nothing yet', async () => {
    m.getMemories.mockResolvedValue({ memories: [], capabilities: caps(true) });
    m.getEpisodes.mockResolvedValue([]);
    const onTalk = vi.fn();
    render(<PartnerMemories agent={agent} onBack={vi.fn()} onTalk={onTalk} />);
    fireEvent.click(await screen.findByRole('button', { name: 'mate.talk' }));
    expect(onTalk).toHaveBeenCalledOnce();
    expect(screen.queryByLabelText('mate.memories_search')).toBeNull();
  });

  it('shows a retry after a failed first load, and keeps loaded memories when a later refresh fails', async () => {
    m.getMemories.mockRejectedValueOnce(new Error('down'));
    m.getEpisodes.mockResolvedValue([episode]);
    render(<PartnerMemories agent={agent} onBack={vi.fn()} onTalk={vi.fn()} />);
    expect(await screen.findByRole('alert')).toHaveTextContent('mate.memories_failed');
    m.getMemories.mockResolvedValue({ memories: [walk], capabilities: caps(true) });
    fireEvent.click(screen.getByRole('button', { name: 'mate.memories_retry' }));
    await screen.findByText(walk.content);

    vi.useFakeTimers();
    try {
      m.getMemories.mockRejectedValue(new Error('down again'));
      for (const handler of m.events) handler({ type: 'MessageReceived' });
      await vi.advanceTimersByTimeAsync(600);
    } finally {
      vi.useRealTimers();
    }
    expect(await screen.findByRole('alert')).toHaveTextContent('mate.memories_refresh_failed');
    expect(screen.getByText(walk.content)).toBeInTheDocument();
  });

  it('says when the kernel page limit may hide older memories', async () => {
    const many = Array.from({ length: 100 }, (_, i) => memory(100 + i, `memory ${i}`, today));
    m.getMemories.mockResolvedValue({ memories: many, capabilities: caps(true) });
    m.getEpisodes.mockResolvedValue([]);
    render(<PartnerMemories agent={agent} onBack={vi.fn()} onTalk={vi.fn()} />);
    expect(await screen.findByText('mate.memories_truncated')).toBeInTheDocument();
  });
});
