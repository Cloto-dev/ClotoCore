import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { McpServerInfo } from '../../../types';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string, o?: Record<string, unknown>) => (o ? `${k}:${Object.values(o).join('|')}` : k),
    i18n: { language: 'en' },
  }),
}));

const data = vi.hoisted(() => ({ servers: [] as McpServerInfo[] }));
const apiFns = vi.hoisted(() => ({ createAgent: vi.fn() }));
vi.mock('../../../hooks/useApi', () => ({ useApi: () => apiFns }));
vi.mock('../../../hooks/useMcpServers', () => ({
  useMcpServers: () => ({ servers: data.servers, isLoading: false, error: null, refetch: vi.fn() }),
}));

import { CreateAgentModal } from '../CreateAgentModal';

function server(id: string, tools: string[]): McpServerInfo {
  return { id, command: 'x', args: [], status: 'Connected', tools, is_cloto_sdk: false } as McpServerInfo;
}

beforeEach(() => {
  vi.clearAllMocks();
  data.servers = [server('ollama', ['think']), server('cpersona', ['recall', 'store'])];
  apiFns.createAgent.mockResolvedValue(undefined);
});

describe('making a new agent', () => {
  it('is a dialog with a name, and the name has the focus when it opens', () => {
    render(<CreateAgentModal onClose={vi.fn()} onCreated={vi.fn()} />);
    const dialog = screen.getByRole('dialog');
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(document.activeElement).toBe(screen.getByLabelText('form.name'));
  });

  it('will not create until it has a name, a description and an engine', () => {
    render(<CreateAgentModal onClose={vi.fn()} onCreated={vi.fn()} />);
    const create = screen.getByText('create');
    expect((create as HTMLButtonElement).disabled).toBe(true);

    fireEvent.change(screen.getByLabelText('form.name'), { target: { value: 'Sapphy' } });
    fireEvent.change(screen.getByLabelText('form.description'), { target: { value: 'Keeps notes.' } });
    expect((create as HTMLButtonElement).disabled).toBe(true);

    fireEvent.click(screen.getByLabelText('form.llm_engine'));
    fireEvent.pointerDown(screen.getByText('ollama'));
    expect((create as HTMLButtonElement).disabled).toBe(false);
  });

  it('creates with what was typed, and closes when the kernel took it', async () => {
    const onCreated = vi.fn();
    render(<CreateAgentModal onClose={vi.fn()} onCreated={onCreated} />);
    fireEvent.change(screen.getByLabelText('form.name'), { target: { value: 'Sapphy' } });
    fireEvent.change(screen.getByLabelText('form.description'), { target: { value: 'Keeps notes.' } });
    fireEvent.click(screen.getByLabelText('form.llm_engine'));
    fireEvent.pointerDown(screen.getByText('ollama'));
    fireEvent.click(screen.getByLabelText('form.memory'));
    fireEvent.pointerDown(screen.getByText('cpersona'));

    fireEvent.click(screen.getByText('create'));
    await waitFor(() => expect(apiFns.createAgent).toHaveBeenCalledTimes(1));
    expect(apiFns.createAgent.mock.calls[0][0]).toEqual({
      name: 'Sapphy',
      description: 'Keeps notes.',
      default_engine: 'ollama',
      metadata: { preferred_memory: 'cpersona', agent_type: 'ai' },
      password: undefined,
    });
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
  });

  it('sends the password and the routing rules only when the advanced part was used', async () => {
    render(<CreateAgentModal onClose={vi.fn()} onCreated={vi.fn()} />);
    fireEvent.change(screen.getByLabelText('form.name'), { target: { value: 'Sapphy' } });
    fireEvent.change(screen.getByLabelText('form.description'), { target: { value: 'Keeps notes.' } });
    fireEvent.click(screen.getByLabelText('form.llm_engine'));
    fireEvent.pointerDown(screen.getByText('ollama'));

    fireEvent.change(screen.getByLabelText('form.password'), { target: { value: 'hunter2' } });
    fireEvent.click(screen.getByText('routing.add_rule'));
    fireEvent.change(screen.getByLabelText('routing.match_label:1'), { target: { value: 'tools_likely' } });
    fireEvent.click(screen.getByLabelText('routing.engine_label:1'));
    const ruleList = screen.getByRole('listbox', { name: 'routing.engine_label:1' });
    fireEvent.pointerDown(within(ruleList).getByRole('option', { name: 'ollama' }));

    fireEvent.click(screen.getByText('create'));
    await waitFor(() => expect(apiFns.createAgent).toHaveBeenCalledTimes(1));
    const payload = apiFns.createAgent.mock.calls[0][0];
    expect(payload.password).toBe('hunter2');
    expect(JSON.parse(payload.metadata.engine_routing)).toEqual([
      { match: 'tools_likely', engine: 'ollama', cfr: true },
    ]);
  });

  it('closes on Escape and on the backdrop, and does not create anything on the way out', () => {
    const onClose = vi.fn();
    const { container } = render(<CreateAgentModal onClose={onClose} onCreated={vi.fn()} />);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);

    const backdrop = container.querySelector('.wmodal-backdrop');
    if (!backdrop) throw new Error('no backdrop');
    fireEvent.click(backdrop);
    expect(onClose).toHaveBeenCalledTimes(2);

    // A click inside the panel is not a click on the way out.
    fireEvent.click(screen.getByRole('dialog'));
    expect(onClose).toHaveBeenCalledTimes(2);
    expect(apiFns.createAgent).not.toHaveBeenCalled();
  });

  it('says so instead of offering a picker when no engine is connected', () => {
    data.servers = [server('cpersona', ['recall'])];
    render(<CreateAgentModal onClose={vi.fn()} onCreated={vi.fn()} />);
    expect(screen.getByText('form.no_engines')).toBeTruthy();
    expect(screen.queryByLabelText('form.llm_engine')).toBeNull();
  });
});
