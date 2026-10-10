import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata } from '../types';
import type { VrmAnimationController } from './engine/VrmAnimationController';
import { MotionControls } from './MotionControls';

const m = vi.hoisted(() => ({
  getAgents: vi.fn(),
  updateAgent: vi.fn(),
  validateModelFile: vi.fn(),
  setPose: vi.fn(),
  setIdleParams: vi.fn(),
  setMotionStyle: vi.fn(),
  previewGesture: vi.fn(),
  stopVrma: vi.fn(),
  loadVrmaAnimationFile: vi.fn(),
  loadVrmaPoseFile: vi.fn(),
}));
vi.mock('../hooks/useApi', () => ({ useApi: () => m }));
vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock('../mate/assetFiles', () => ({ validateModelFile: m.validateModelFile }));
vi.mock('../mate/usePartnerMedia', () => ({ partnerSettingsChanged: vi.fn() }));
vi.mock('./motionLibrary', async (original) => ({ ...(await original<object>()), listMotions: async () => [] }));
const agent = {
  id: 'agent.mio',
  name: 'Mio',
  description: '',
  required_capabilities: [],
  enabled: true,
  last_seen: 0,
  status: 'online',
  metadata: { accent: 'old', mate_motion_style: 'legacy' },
} as AgentMetadata;
const controller = { current: m as unknown as VrmAnimationController };
beforeEach(() => vi.resetAllMocks());
describe('motion controls', () => {
  it('previews without writing and saves using the latest complete metadata', async () => {
    m.getAgents.mockResolvedValue([
      {
        ...agent,
        metadata: {
          accent: 'new',
          voice: 'keep',
          has_vrm: 'true',
          vrm_path: '/model.vrm',
          has_avatar: 'true',
          avatar_path: '/icon.png',
        },
      },
    ]);
    const onSaved = vi.fn();
    const view = render(<MotionControls agent={agent} controller={controller} ready onSaved={onSaved} />);
    fireEvent.click(screen.getByRole('button', { name: 'mate.poses.attentive' }));
    expect(m.setPose).toHaveBeenCalledWith('attentive');
    expect(m.updateAgent).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'mate.save_motion' }));
    await waitFor(() => expect(onSaved).toHaveBeenCalledOnce());
    expect(m.updateAgent).toHaveBeenCalledWith(agent.id, {
      metadata: {
        accent: 'new',
        voice: 'keep',
        mate_motion_style: 'legacy',
        mate_idle: expect.any(String),
        mate_pose: 'attentive',
      },
    });
    const saved = JSON.parse(m.updateAgent.mock.calls[0][1].metadata.mate_idle);
    expect(saved.pose.spine_x).toBe(0.05);
    expect(saved.breathing_rate).toBe(1);
    view.rerender(
      <MotionControls
        agent={{ ...agent, metadata: m.updateAgent.mock.calls[0][1].metadata }}
        controller={controller}
        ready
        onSaved={onSaved}
      />,
    );
    expect(screen.queryByText('mate.saved')).toBeVisible();
  });
  it('leaves other settings intact when the pre-save read fails', async () => {
    m.getAgents.mockRejectedValue(new Error('Offline'));
    render(<MotionControls agent={agent} controller={controller} ready onSaved={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: 'mate.save_motion' }));
    await screen.findByRole('alert');
    expect(m.updateAgent).not.toHaveBeenCalled();
  });
  it('loads a VRMA as animation or first-frame pose according to the selected mode', async () => {
    render(<MotionControls agent={agent} controller={controller} ready onSaved={vi.fn()} />);
    const file = new File(['motion'], 'sample.vrma');
    fireEvent.change(screen.getByLabelText('mate.choose_motion'), { target: { files: [file] } });
    await waitFor(() => expect(m.loadVrmaAnimationFile).toHaveBeenCalledWith(file));
    await waitFor(() => expect(screen.getByRole('button', { name: 'mate.save_motion' })).toBeEnabled());
    fireEvent.change(screen.getByLabelText('mate.play_as'), { target: { value: 'pose' } });
    fireEvent.change(screen.getByLabelText('mate.choose_motion'), { target: { files: [file] } });
    await waitFor(() => expect(m.loadVrmaPoseFile).toHaveBeenCalledWith(file));
    expect(m.validateModelFile).toHaveBeenCalledWith(file, 'motion');
    expect(m.updateAgent).not.toHaveBeenCalled();
  });
  it('saves new movement independently without changing the saved original motion', async () => {
    m.getAgents.mockResolvedValue([
      { ...agent, metadata: { mate_idle: 'original idle', mate_pose: 'thinking', voice: 'keep' } },
    ]);
    const onSaved = vi.fn();
    render(<MotionControls agent={agent} controller={controller} ready onSaved={onSaved} />);
    fireEvent.change(screen.getByLabelText('mate.motion_style'), { target: { value: 'companion' } });
    expect(m.setMotionStyle).toHaveBeenCalledWith('companion');
    fireEvent.click(screen.getByRole('button', { name: 'mate.gestures.greet' }));
    expect(m.previewGesture).toHaveBeenCalledWith('greet');
    fireEvent.click(screen.getByRole('button', { name: 'mate.poses.attentive' }));
    expect(m.updateAgent).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'mate.save_motion' }));
    await waitFor(() => expect(onSaved).toHaveBeenCalledOnce());
    expect(m.updateAgent).toHaveBeenCalledWith(agent.id, {
      metadata: {
        mate_idle: 'original idle',
        mate_pose: 'thinking',
        voice: 'keep',
        mate_motion_style: 'companion',
        mate_companion_idle: expect.any(String),
        mate_companion_pose: 'attentive',
      },
    });
  });
  it('keeps each style draft when switching for comparison without saving', () => {
    render(<MotionControls agent={agent} controller={controller} ready onSaved={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: 'mate.poses.attentive' }));
    fireEvent.change(screen.getByLabelText('mate.motion_style'), { target: { value: 'companion' } });
    fireEvent.click(screen.getByRole('button', { name: 'mate.poses.arms_crossed' }));
    fireEvent.change(screen.getByLabelText('mate.motion_style'), { target: { value: 'legacy' } });
    expect(screen.getByRole('button', { name: 'mate.poses.attentive' })).toHaveAttribute('aria-pressed', 'true');
    expect(m.setPose).toHaveBeenLastCalledWith('attentive');
    expect(m.updateAgent).not.toHaveBeenCalled();
  });
});
