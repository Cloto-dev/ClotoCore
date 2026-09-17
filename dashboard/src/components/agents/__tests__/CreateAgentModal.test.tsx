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
const apiFns = vi.hoisted(() => ({ createAgent: vi.fn(), uploadAvatar: vi.fn(), uploadVrm: vi.fn() }));
const vrm = vi.hoisted(() => ({ extractVrmThumbnail: vi.fn() }));
vi.mock('../../../lib/vrmThumbnail', () => vrm);
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
  apiFns.createAgent.mockResolvedValue({ id: 'agent.sapphy' });
  apiFns.uploadAvatar.mockResolvedValue({});
  apiFns.uploadVrm.mockResolvedValue({});
  vrm.extractVrmThumbnail.mockResolvedValue(null);
  sessionStorage.clear();
  let n = 0;
  URL.createObjectURL = vi.fn(() => `blob:face-${++n}`);
  URL.revokeObjectURL = vi.fn();
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

function fillRequired() {
  fireEvent.change(screen.getByLabelText('form.name'), { target: { value: 'Sapphy' } });
  fireEvent.change(screen.getByLabelText('form.description'), { target: { value: 'Keeps notes.' } });
  fireEvent.click(screen.getByLabelText('form.llm_engine'));
  fireEvent.pointerDown(screen.getByText('ollama'));
}

function fileInput(container: HTMLElement, accept: string): HTMLInputElement {
  const input = container.querySelector(`input[type="file"][accept="${accept}"]`);
  if (!input) throw new Error(`no file input for ${accept}`);
  return input as HTMLInputElement;
}

const png = (bytes = 10) => new File([new Uint8Array(bytes)], 'face.png', { type: 'image/png' });
const model = () => new File([new Uint8Array(10)], 'body.vrm', { type: 'model/gltf-binary' });

describe('giving the new agent a face', () => {
  it('shows the chosen picture at once and uploads nothing until the agent exists', async () => {
    const onCreated = vi.fn();
    const { container } = render(<CreateAgentModal onClose={vi.fn()} onCreated={onCreated} />);
    fillRequired();
    const face = png();
    fireEvent.change(fileInput(container, 'image/*'), { target: { files: [face] } });
    expect(screen.getByTestId('create-face').querySelector('img')?.getAttribute('src')).toBe('blob:face-1');
    expect(apiFns.uploadAvatar).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText('create'));
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    // To the agent the kernel said it made, and only after it made it.
    expect(apiFns.uploadAvatar).toHaveBeenCalledWith('agent.sapphy', face);
    expect(apiFns.createAgent.mock.invocationCallOrder[0]).toBeLessThan(
      apiFns.uploadAvatar.mock.invocationCallOrder[0],
    );
    expect(apiFns.uploadVrm).not.toHaveBeenCalled();
    expect(onCreated).toHaveBeenCalledWith({ name: 'Sapphy', id: 'agent.sapphy', faceProblem: null });
  });

  it('uploads nothing when no picture and no model were chosen', async () => {
    const onCreated = vi.fn();
    render(<CreateAgentModal onClose={vi.fn()} onCreated={onCreated} />);
    fillRequired();
    fireEvent.click(screen.getByText('create'));
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    expect(apiFns.uploadAvatar).not.toHaveBeenCalled();
    expect(apiFns.uploadVrm).not.toHaveBeenCalled();
  });

  it('forgets a picture that was taken back', async () => {
    const onCreated = vi.fn();
    const { container } = render(<CreateAgentModal onClose={vi.fn()} onCreated={onCreated} />);
    fillRequired();
    fireEvent.change(fileInput(container, 'image/*'), { target: { files: [png()] } });
    fireEvent.click(screen.getByText('plugin_workspace.avatar_remove'));
    expect(screen.getByTestId('create-face').querySelector('img')).toBeNull();
    fireEvent.click(screen.getByText('create'));
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    expect(apiFns.uploadAvatar).not.toHaveBeenCalled();
  });

  it('refuses a picture the kernel would refuse, before anything is sent', async () => {
    const onCreated = vi.fn();
    const { container } = render(<CreateAgentModal onClose={vi.fn()} onCreated={onCreated} />);
    fillRequired();
    fireEvent.change(fileInput(container, 'image/*'), { target: { files: [png(5 * 1024 * 1024 + 1)] } });
    expect(screen.getByText('plugin_workspace.avatar_too_large')).toBeTruthy();
    expect(screen.getByTestId('create-face').querySelector('img')).toBeNull();
    fireEvent.click(screen.getByText('create'));
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    expect(apiFns.uploadAvatar).not.toHaveBeenCalled();
  });

  it('accepts a picture of exactly the largest size', () => {
    const { container } = render(<CreateAgentModal onClose={vi.fn()} onCreated={vi.fn()} />);
    fireEvent.change(fileInput(container, 'image/*'), { target: { files: [png(5 * 1024 * 1024)] } });
    expect(screen.queryByText('plugin_workspace.avatar_too_large')).toBeNull();
    expect(screen.getByTestId('create-face').querySelector('img')).not.toBeNull();
  });

  it('sends the model after the picture, to the same agent', async () => {
    const onCreated = vi.fn();
    const { container } = render(<CreateAgentModal onClose={vi.fn()} onCreated={onCreated} />);
    fillRequired();
    const body = model();
    fireEvent.change(fileInput(container, 'image/*'), { target: { files: [png()] } });
    fireEvent.change(fileInput(container, '.vrm'), { target: { files: [body] } });
    await screen.findByText('body.vrm');
    fireEvent.click(screen.getByText('create'));
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    expect(apiFns.uploadVrm).toHaveBeenCalledWith('agent.sapphy', body);
    expect(apiFns.uploadAvatar.mock.invocationCallOrder[0]).toBeLessThan(apiFns.uploadVrm.mock.invocationCallOrder[0]);
  });

  it('offers the thumbnail a model carries, and uses it as the picture when taken', async () => {
    const thumb = new File([new Uint8Array(4)], 'thumb.png', { type: 'image/png' });
    vrm.extractVrmThumbnail.mockResolvedValue(thumb);
    const onCreated = vi.fn();
    const { container } = render(<CreateAgentModal onClose={vi.fn()} onCreated={onCreated} />);
    fillRequired();
    fireEvent.change(fileInput(container, '.vrm'), { target: { files: [model()] } });
    fireEvent.click(await screen.findByText('plugin_workspace.vrm_thumbnail_apply'));
    fireEvent.click(screen.getByText('create'));
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    expect(apiFns.uploadAvatar).toHaveBeenCalledWith('agent.sapphy', thumb);
  });

  it('does not call a saved agent a failed creation when only its picture could not be saved', async () => {
    apiFns.uploadAvatar.mockRejectedValue(new Error('disk full'));
    const onCreated = vi.fn();
    const { container } = render(<CreateAgentModal onClose={vi.fn()} onCreated={onCreated} />);
    fillRequired();
    fireEvent.change(fileInput(container, 'image/*'), { target: { files: [png()] } });
    fireEvent.click(screen.getByText('create'));
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    expect(onCreated).toHaveBeenCalledWith({ name: 'Sapphy', id: 'agent.sapphy', faceProblem: 'disk full' });
    // Nothing invites a second Create, which would ask for a second Sapphy.
    expect(screen.queryByText('disk full')).toBeNull();
    expect(apiFns.createAgent).toHaveBeenCalledTimes(1);
  });

  it('says so when the kernel did not name the agent it made, rather than guessing one', async () => {
    apiFns.createAgent.mockResolvedValue({ id: null });
    const onCreated = vi.fn();
    const { container } = render(<CreateAgentModal onClose={vi.fn()} onCreated={onCreated} />);
    fillRequired();
    fireEvent.change(fileInput(container, 'image/*'), { target: { files: [png()] } });
    fireEvent.click(screen.getByText('create'));
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    expect(apiFns.uploadAvatar).not.toHaveBeenCalled();
    expect(onCreated.mock.calls[0][0].faceProblem).toMatch(/did not say which agent/);
  });
});
