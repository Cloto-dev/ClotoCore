import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata } from '../types';
import { PartnerSettings } from './PartnerSettings';

const m = vi.hoisted(() => ({
  uploadAvatar: vi.fn(),
  uploadVrm: vi.fn(),
  updateAgent: vi.fn(),
  refetchAgents: vi.fn(),
  validateIconFile: vi.fn(),
  validateModelFile: vi.fn(),
  loadFile: vi.fn(),
  dispose: vi.fn(),
  changed: vi.fn(),
}));
vi.mock('../hooks/useApi', () => ({ useApi: () => m }));
vi.mock('../contexts/AgentContext', () => ({ useAgentContext: () => m }));
vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock('./assetFiles', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./assetFiles')>()),
  validateIconFile: m.validateIconFile,
  validateModelFile: m.validateModelFile,
}));
vi.mock('./usePartnerMedia', () => ({ partnerMediaChanged: m.changed }));
vi.mock('../vrm/engine/VrmModelLoader', () => ({
  VrmModelLoader: class {
    loadFile = m.loadFile;
    dispose = m.dispose;
  },
}));
const agent = {
  id: 'agent.mio',
  name: 'Mio',
  description: '',
  required_capabilities: [],
  enabled: true,
  last_seen: 0,
  status: 'online',
  metadata: {},
} as AgentMetadata;
beforeEach(() => {
  vi.resetAllMocks();
});
describe('saved partner settings', () => {
  it('uploads an icon immediately while leaving a draft name unapplied on back', async () => {
    const back = vi.fn();
    render(<PartnerSettings agent={agent} onBack={back} />);
    fireEvent.change(screen.getByLabelText('mate.name'), { target: { value: 'Draft' } });
    const file = new File(['image'], 'icon.png', { type: 'image/png' });
    fireEvent.change(screen.getByLabelText('mate.choose_icon'), { target: { files: [file] } });
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('mate.saved'));
    expect(m.uploadAvatar).toHaveBeenCalledWith(agent.id, file);
    expect(m.refetchAgents).toHaveBeenCalled();
    expect(m.updateAgent).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'mate.back' }));
    expect(back).toHaveBeenCalledOnce();
    expect(m.updateAgent).not.toHaveBeenCalled();
  });
  it('applies the trimmed name only through its explicit button', async () => {
    render(<PartnerSettings agent={agent} onBack={vi.fn()} />);
    fireEvent.change(screen.getByLabelText('mate.name'), { target: { value: '  Mio Two  ' } });
    expect(m.updateAgent).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'mate.apply_name' }));
    await waitFor(() => expect(m.updateAgent).toHaveBeenCalledWith(agent.id, { name: 'Mio Two' }));
  });
  it('does not replace the stored VRM when parsing the new file fails', async () => {
    m.loadFile.mockRejectedValue(new Error('Invalid rig'));
    render(<PartnerSettings agent={agent} onBack={vi.fn()} />);
    fireEvent.change(screen.getByLabelText('mate.choose_vrm'), { target: { files: [new File(['bad'], 'bad.vrm')] } });
    await screen.findByRole('alert');
    expect(m.dispose).toHaveBeenCalledOnce();
    expect(m.uploadVrm).not.toHaveBeenCalled();
    expect(m.changed).not.toHaveBeenCalled();
    expect(m.refetchAgents).not.toHaveBeenCalled();
  });
});
