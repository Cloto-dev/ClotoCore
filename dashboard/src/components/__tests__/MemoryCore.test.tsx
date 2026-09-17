import { readFileSync } from 'node:fs';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata, Episode, Memory } from '../../types';

// The real English strings, so an assertion reads as what a person sees and a
// key that does not exist fails here rather than rendering its own name.
vi.mock('react-i18next', async () => {
  const en = (await import('../../locales/en/memory.json')).default as Record<string, string>;
  return {
    useTranslation: () => ({
      t: (key: string, vars?: Record<string, unknown>) =>
        (en[key] ?? key).replace(/\{\{(\w+)\}\}/g, (_m, name) => String(vars?.[name] ?? '')),
      i18n: { language: 'en' },
    }),
  };
});

// One object, not a new one per render: the screen's fetch is a useCallback
// keyed on these functions, so fresh identities would re-fetch for ever.
const api = vi.hoisted(() => ({
  apiKey: 'test-key',
  getMemories: vi.fn(),
  getEpisodes: vi.fn(),
  getAgents: vi.fn(),
  deleteMemory: vi.fn(),
  deleteEpisode: vi.fn(),
  updateMemory: vi.fn(),
  lockMemory: vi.fn(),
  unlockMemory: vi.fn(),
  importMemories: vi.fn(),
}));
vi.mock('../../hooks/useApi', () => ({ useApi: () => api }));

const metrics = vi.hoisted(() => ({
  metrics: { ram_usage: '42 MB', total_memories: 3, total_requests: 0, total_episodes: 0 },
}));
vi.mock('../../hooks/useMetrics', () => ({ useMetrics: () => metrics }));
vi.mock('../../hooks/useEventStream', () => ({ useEventStream: () => undefined }));

const agentCtx = vi.hoisted(() => ({ selectedAgentId: null as string | null }));
vi.mock('../../contexts/AgentContext', () => ({ useAgentContext: () => agentCtx }));

import { MemoryCore } from '../MemoryCore';

const KARIN = 'agent.karin';
const SAPPHY = 'agent.sapphy';

const agent = (id: string, name: string): AgentMetadata =>
  ({
    id,
    name,
    description: '',
    required_capabilities: [],
    enabled: true,
    last_seen: 0,
    status: 'online',
    metadata: {},
  }) as AgentMetadata;

/** A local moment, handed over the way the memory server hands one over. */
function at(daysAgo: number, hour: number): string {
  const now = new Date();
  return new Date(now.getFullYear(), now.getMonth(), now.getDate() - daysAgo, hour, 0).toISOString();
}

const memory = (id: number, agentId: string, content: string, when: string, over: Partial<Memory> = {}): Memory => ({
  id,
  agent_id: agentId,
  content,
  source: { type: 'Agent' },
  timestamp: when,
  created_at: when,
  ...over,
});

const episode = (id: number, agentId: string, summary: string): Episode => ({
  id,
  agent_id: agentId,
  summary,
  keywords: 'greeting, cron',
  created_at: at(1, 22),
  end_time: at(1, 22),
});

// Today two memories, then nothing for three whole days, then one more.
const MEMORIES = [
  memory(1, SAPPHY, 'the morning greeting can wait; the recall gate is the thing to move', at(0, 14)),
  memory(2, KARIN, 'Discord has no send tool, so cron carries it', at(0, 8)),
  memory(3, KARIN, 'three drafts of the morning greeting', at(4, 22)),
];

function setData(memories: Memory[] = MEMORIES, episodes: Episode[] = [], updatable = true) {
  api.getMemories.mockResolvedValue({
    memories,
    capabilities: {
      update_memory: updatable,
      lock_memory: true,
      unlock_memory: true,
      set_recall_precision: false,
      get_recall_precision: false,
    },
  });
  api.getEpisodes.mockResolvedValue(episodes);
  api.getAgents.mockResolvedValue([agent(KARIN, 'Karin'), agent(SAPPHY, 'Sapphy')]);
}

async function mount() {
  const view = render(<MemoryCore />);
  await screen.findByTestId('memory-density');
  return view;
}

const page = () => screen.getByTestId('memory-page');
const rowsOnAxis = () => Array.from(page().querySelectorAll<HTMLElement>('.tl .ev'));
const bodies = () => rowsOnAxis().map((r) => r.querySelector('.tx')?.textContent ?? '');
const rowFor = (id: number) => screen.getByTestId(`memory-row-m${id}`);
const act = (row: HTMLElement, label: string) => {
  const button = Array.from(row.querySelectorAll('button')).find((b) => b.textContent === label);
  if (!button) throw new Error(`no "${label}" on this row`);
  return button;
};

beforeEach(() => {
  vi.clearAllMocks();
  agentCtx.selectedAgentId = null;
  setData();
  api.deleteMemory.mockResolvedValue(undefined);
  api.deleteEpisode.mockResolvedValue(undefined);
  api.updateMemory.mockResolvedValue(undefined);
  api.lockMemory.mockResolvedValue({ lock_level: 'kernel' });
  api.unlockMemory.mockResolvedValue({ lock_level: 'kernel' });
  api.importMemories.mockResolvedValue({ imported_memories: 2, imported_episodes: 0, skipped_memories: 0 });
  vi.spyOn(window, 'confirm').mockReturnValue(true);
  vi.spyOn(window, 'alert').mockImplementation(() => undefined);
});

describe('the memory screen as a time axis', () => {
  it('draws a band per day, a row per memory and the hole between them', async () => {
    await mount();
    expect(page().querySelector('.ws-head .count')?.textContent).toBe('3 long-term, 0 episodes');
    expect(page().querySelectorAll('.tl .tday')).toHaveLength(2);
    expect(rowsOnAxis()).toHaveLength(3);
    expect(page().querySelector('.tl .tday .lbl')?.textContent).toBe('Today');
    expect(page().querySelector('.tl .tday .n')?.textContent).toBe('2 memories');
    expect(screen.getByText('3 days, no memories')).toBeTruthy();
    expect(page().querySelectorAll('.tl .gap')).toHaveLength(1);
  });

  it('gives the point the agent colour only for the agent who is present', async () => {
    agentCtx.selectedAgentId = SAPPHY;
    await mount();
    const mine = rowsOnAxis().filter((r) => r.classList.contains('mine'));
    expect(mine).toHaveLength(1);
    expect(mine[0].querySelector('.tx')?.textContent).toBe(
      'the morning greeting can wait; the recall gate is the thing to move',
    );
    expect(rowsOnAxis().filter((r) => !r.classList.contains('mine'))).toHaveLength(2);
  });

  it('gives nobody the agent colour when nobody is present', async () => {
    await mount();
    expect(rowsOnAxis().filter((r) => r.classList.contains('mine'))).toHaveLength(0);
  });

  it('sets what was remembered in the reading face, and only the clock in monospace', async () => {
    await mount();
    const body = rowsOnAxis()[0].querySelector('.tx') as HTMLElement;
    expect(body.className).toBe('tx');
    expect(page().querySelectorAll('.tl .font-mono')).toHaveLength(0);
    // The clock is the tabular one (`.num` is the workshop's monospace class).
    expect(rowsOnAxis()[0].querySelector('.t')?.className).toContain('num');
    const css = readFileSync('src/components/Memory.css', 'utf8');
    const rule = css.slice(css.indexOf('.tl .ev .tx {'), css.indexOf('}', css.indexOf('.tl .ev .tx {')));
    expect(rule).not.toContain('mono');
    expect(rule).toContain('-webkit-line-clamp: 2');
  });

  it('opens a row for the keyboard, not only for the pointer', async () => {
    await mount();
    for (const row of rowsOnAxis()) {
      expect(row.querySelector('.c')?.getAttribute('tabindex')).toBe('0');
    }
    const css = readFileSync('src/components/Memory.css', 'utf8');
    expect(css).toContain('.tl .ev .c:focus .tx');
    expect(css).toContain('.tl .ev .c:focus-within .tx');
  });
});

describe('narrowing what is on the axis', () => {
  it('makes the agent tab and the search narrow together, not either on its own', async () => {
    await mount();
    fireEvent.click(screen.getByRole('button', { name: 'Karin' }));
    await waitFor(() => expect(rowsOnAxis()).toHaveLength(2));

    fireEvent.change(screen.getByLabelText('Search the text, an agent or a date'), { target: { value: 'greeting' } });
    // Karin AND "greeting". Either half on its own would keep two rows —
    // Karin has two memories, and "greeting" is in Sapphy's newest — so a
    // screen that took either half would show more than this one.
    await waitFor(() => expect(bodies()).toEqual(['three drafts of the morning greeting']));

    fireEvent.click(screen.getByRole('button', { name: 'All' }));
    await waitFor(() =>
      expect(bodies()).toEqual([
        'the morning greeting can wait; the recall gate is the thing to move',
        'three drafts of the morning greeting',
      ]),
    );
  });

  it('searches the agent name as well as the text', async () => {
    await mount();
    fireEvent.change(screen.getByLabelText('Search the text, an agent or a date'), { target: { value: 'sapphy' } });
    await waitFor(() =>
      expect(bodies()).toEqual(['the morning greeting can wait; the recall gate is the thing to move']),
    );
  });

  it('says nothing matches, not "nothing is remembered", under an agent who has none', async () => {
    // The tab scopes the fetch, so the answer really is empty — but it is
    // empty because of the filter, and the screen has to say which.
    await mount();
    api.getMemories.mockResolvedValue({
      memories: [],
      capabilities: {
        update_memory: true,
        lock_memory: true,
        unlock_memory: true,
        set_recall_precision: false,
        get_recall_precision: false,
      },
    });
    api.getEpisodes.mockResolvedValue([]);
    fireEvent.click(screen.getByRole('button', { name: 'Sapphy' }));
    await screen.findByText('Nothing here matches what you are looking for.');
  });

  it('says so when the filters leave nothing', async () => {
    await mount();
    fireEvent.change(screen.getByLabelText('Search the text, an agent or a date'), { target: { value: 'zzz' } });
    await screen.findByText('Nothing here matches what you are looking for.');
    expect(rowsOnAxis()).toHaveLength(0);
  });

  it('puts the episodes on the axis when only episodes are wanted', async () => {
    setData(MEMORIES, [episode(7, KARIN, 'settled the morning greeting')]);
    await mount();
    expect(rowsOnAxis()).toHaveLength(3);
    fireEvent.click(screen.getByRole('button', { name: 'Episodes only' }));
    await waitFor(() => expect(bodies()).toEqual(['settled the morning greeting']));
    // and not twice: the panel on the right stands down while they are here
    expect(page().querySelectorAll('.mem-ep')).toHaveLength(0);
  });
});

describe('what the screen says when it has nothing to show', () => {
  it('says what makes a memory when there is not one', async () => {
    setData([], []);
    await mount();
    expect(
      screen.getByText(
        'Nothing is remembered yet. What an agent keeps from your conversations appears here, newest first.',
      ),
    ).toBeTruthy();
  });

  it('teaches how the first episode is made', async () => {
    setData(MEMORIES, []);
    await mount();
    expect(
      screen.getByText('None yet. When a conversation comes to a stop, the agent leaves a summary of it here.'),
    ).toBeTruthy();
    expect(screen.getByText('The first one is made by saying "that\'s it for now" in the chat.')).toBeTruthy();
  });

  it('offers the way back when the fetch failed', async () => {
    api.getMemories.mockRejectedValue(new Error('no memory server'));
    render(<MemoryCore />);
    await screen.findByText('Operation failed');
    api.getMemories.mockClear();
    setData();
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(api.getMemories).toHaveBeenCalled());
    await screen.findByText('Today');
  });
});

describe('the density band', () => {
  it('draws thirty days ending today and counts what is on them', async () => {
    await mount();
    const cells = Array.from(screen.getByTestId('memory-density').querySelectorAll('.d'));
    expect(cells).toHaveLength(30);
    // Newest first, as the mock draws it.
    expect(cells[0].classList.contains('today')).toBe(true);
    expect(cells[0].querySelector('i')?.getAttribute('title')).toBe('2 memories');
    expect(cells[4].querySelector('i')?.getAttribute('title')).toBe('1 memories');
    const total = cells.reduce((sum, c) => sum + Number(c.querySelector('i')?.getAttribute('title')?.split(' ')[0]), 0);
    expect(total).toBe(MEMORIES.length);
  });

  it('colours a day only where the agent who is present has a memory', async () => {
    agentCtx.selectedAgentId = SAPPHY;
    await mount();
    const cells = Array.from(screen.getByTestId('memory-density').querySelectorAll('.d'));
    expect(cells.filter((c) => c.classList.contains('mine'))).toHaveLength(1);
    expect(cells[0].classList.contains('mine')).toBe(true);
  });
});

describe('what a row can still be told to do', () => {
  it('deletes through the same call, with no question asked first', async () => {
    await mount();
    fireEvent.click(act(rowFor(2), 'Delete'));
    await waitFor(() => expect(api.deleteMemory).toHaveBeenCalledWith(2));
    expect(window.confirm).not.toHaveBeenCalled();
    await waitFor(() => expect(rowsOnAxis()).toHaveLength(2));
  });

  it('refuses to delete a locked memory and says why', async () => {
    setData([memory(9, KARIN, 'kept on purpose', at(0, 9), { locked: true, lock_level: 'kernel' })]);
    await mount();
    const button = act(rowFor(9), 'Delete');
    expect(button.hasAttribute('disabled')).toBe(true);
    expect(button.getAttribute('title')).toBe('Locked memories cannot be deleted');
    fireEvent.click(button);
    expect(api.deleteMemory).not.toHaveBeenCalled();
  });

  it('locks and unlocks through the calls it always used', async () => {
    await mount();
    fireEvent.click(act(rowFor(1), 'Lock'));
    await waitFor(() => expect(api.lockMemory).toHaveBeenCalledWith(1));
    await waitFor(() => expect(act(rowFor(1), 'Unlock')).toBeTruthy());
    fireEvent.click(act(rowFor(1), 'Unlock'));
    await waitFor(() => expect(api.unlockMemory).toHaveBeenCalledWith(1));
  });

  it('edits only where the server allows it, and saves what was written', async () => {
    await mount();
    fireEvent.click(act(rowFor(1), 'Edit'));
    const box = screen.getByLabelText('Edit') as HTMLTextAreaElement;
    expect(box.value).toBe('the morning greeting can wait; the recall gate is the thing to move');
    fireEvent.change(box, { target: { value: 'the recall gate stays where it is' } });
    fireEvent.click(act(rowFor(1), 'Save'));
    await waitFor(() => expect(api.updateMemory).toHaveBeenCalledWith(1, 'the recall gate stays where it is'));
  });

  it('offers no edit when the memory server cannot take one', async () => {
    setData(MEMORIES, [], false);
    await mount();
    expect(Array.from(rowFor(1).querySelectorAll('button')).map((b) => b.textContent)).toEqual(['Lock', 'Delete']);
  });

  it('deletes an episode from the panel through the same call', async () => {
    setData(MEMORIES, [episode(7, KARIN, 'settled the morning greeting')]);
    await mount();
    const panel = page().querySelector('.mem-ep') as HTMLElement;
    fireEvent.click(panel.querySelector('button') as HTMLButtonElement);
    await waitFor(() => expect(api.deleteEpisode).toHaveBeenCalledWith(7));
  });

  it('asks before it imports, and sends the file as it was read', async () => {
    await mount();
    const line = JSON.stringify({ _type: 'memory', id: 1, content: 'from a file' });
    const file = new File([line], 'memories.jsonl', { type: 'application/x-ndjson' });
    fireEvent.change(screen.getByLabelText('Import'), { target: { files: [file] } });
    await waitFor(() => expect(api.importMemories).toHaveBeenCalledWith(line, ''));
    expect(window.confirm).toHaveBeenCalledWith('Import 1 memories and 0 episodes?');
  });

  it('sends nothing when the question about importing is answered no', async () => {
    vi.mocked(window.confirm).mockReturnValue(false);
    await mount();
    const file = new File(['{"_type":"memory"}'], 'memories.jsonl', { type: 'application/x-ndjson' });
    fireEvent.change(screen.getByLabelText('Import'), { target: { files: [file] } });
    await waitFor(() => expect(window.confirm).toHaveBeenCalled());
    expect(api.importMemories).not.toHaveBeenCalled();
  });

  it('fetches again when asked to', async () => {
    await mount();
    api.getMemories.mockClear();
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(api.getMemories).toHaveBeenCalledTimes(1));
  });

  it('scopes the fetch to the agent whose tab is chosen', async () => {
    await mount();
    fireEvent.click(screen.getByRole('button', { name: 'Karin' }));
    await waitFor(() => expect(api.getMemories).toHaveBeenLastCalledWith(KARIN));
    expect(api.getEpisodes).toHaveBeenLastCalledWith(KARIN);
  });
});
