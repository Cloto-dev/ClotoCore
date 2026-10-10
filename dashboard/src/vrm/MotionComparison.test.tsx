import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata } from '../types';
import type { VrmAnimationController } from './engine/VrmAnimationController';
import { MotionComparison } from './MotionComparison';
import { type MotionEntry, motionReport } from './motionLibrary';

const m = vi.hoisted(() => ({
  list: vi.fn(),
  save: vi.fn(),
  review: vi.fn(),
  remove: vi.fn(),
  parse: vi.fn(),
  load: vi.fn(),
  pause: vi.fn(),
  seek: vi.fn(),
  speed: vi.fn(),
  stop: vi.fn(),
}));
vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock('./motionLibrary', async (original) => ({
  ...(await original<object>()),
  listMotions: m.list,
  saveMotion: m.save,
  saveMotionReview: m.review,
  removeMotion: m.remove,
}));
vi.mock('./engine/MotionImport', async (original) => ({ ...(await original<object>()), importMotion: m.parse }));
const agent = { id: 'mio', name: 'Mio' } as AgentMetadata;
const row: MotionEntry = {
  id: 'clip-a',
  name: 'Greeting',
  source: 'Kimodo',
  conditions: 'Seed 42',
  filename: 'greet.bvh',
  file: new Blob(['data']),
  sha256: 'hash',
  duration: 2,
  bones: 30,
  fps: 30,
  added: '2026-10-09',
  reviews: {},
};
let state = { active: true, paused: false, time: 0.5, duration: 2, speed: 1 };
const target = {
  loadComparisonMotion: m.load,
  pauseMotion: m.pause,
  seekMotion: m.seek,
  setMotionSpeed: m.speed,
  stopVrma: m.stop,
  get isVrmaActive() {
    return true;
  },
  get motionPlayback() {
    return { ...state };
  },
} as unknown as VrmAnimationController;
const controller = { current: target };
function Host({ ready = true }: { ready?: boolean }) {
  const [motion, setMotion] = useState<string | null>(null);
  return <MotionComparison agent={agent} controller={controller} ready={ready} motion={motion} onStarted={setMotion} />;
}
beforeEach(() => {
  vi.resetAllMocks();
  state = { active: true, paused: false, time: 0.5, duration: 2, speed: 1 };
  m.list.mockResolvedValue([row]);
  m.load.mockResolvedValue({});
  m.pause.mockImplementation((value) => {
    state.paused = value;
  });
  m.seek.mockImplementation((value) => {
    state.time = value;
  });
  m.speed.mockImplementation((value) => {
    state.speed = value;
  });
});
async function choose() {
  await screen.findByText('Kimodo · Greeting');
  fireEvent.change(screen.getByLabelText('mate.lab.selection'), { target: { value: row.id } });
}

describe('motion comparison', () => {
  it('requires explicit play, pauses and steps by the chosen inspection rate', async () => {
    render(<Host />);
    await choose();
    expect(m.load).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'mate.lab.play' }));
    await screen.findByRole('button', { name: 'mate.lab.pause' });
    expect(m.load.mock.calls[0][0].name).toBe('greet.bvh');
    fireEvent.click(screen.getByRole('button', { name: 'mate.lab.pause' }));
    expect(m.pause).toHaveBeenLastCalledWith(true);
    fireEvent.change(screen.getByLabelText('mate.lab.fps'), { target: { value: '60' } });
    fireEvent.click(screen.getByRole('button', { name: 'mate.lab.next_frame' }));
    expect(m.seek).toHaveBeenLastCalledWith(0.5 + 1 / 60);
    fireEvent.change(screen.getByLabelText('mate.lab.speed'), { target: { value: '0.5' } });
    expect(m.speed).toHaveBeenLastCalledWith(0.5);
    fireEvent.click(screen.getByRole('button', { name: 'mate.stop_motion' }));
    expect(m.stop).toHaveBeenCalledOnce();
    expect(screen.queryByLabelText('mate.lab.position')).not.toBeInTheDocument();
  });
  it('keeps library selection disabled for playback when no avatar is ready', async () => {
    render(<Host ready={false} />);
    await choose();
    expect(screen.getByRole('button', { name: 'mate.lab.play' })).toBeDisabled();
    expect(screen.getByLabelText('mate.lab.register')).toBeEnabled();
  });
  it('saves observations against the selected avatar and exports metadata without the file blob', async () => {
    render(<Host />);
    await choose();
    fireEvent.change(screen.getByLabelText('mate.lab.review'), { target: { value: 'Foot slides; fingers absent' } });
    fireEvent.click(screen.getByRole('button', { name: 'mate.lab.save_review' }));
    await waitFor(() =>
      expect(m.review).toHaveBeenCalledWith('clip-a', 'mio', 'Mio', 'Foot slides; fingers absent', 30),
    );
    const report = JSON.parse(motionReport([row]));
    expect(report.motions[0]).toMatchObject({ source: 'Kimodo', conditions: 'Seed 42', sha256: 'hash', bytes: 4 });
    expect(report.motions[0]).not.toHaveProperty('file');
  });
  it('does not register a failed import or announce success after a storage failure', async () => {
    render(<Host />);
    await choose();
    m.parse.mockRejectedValue(new Error('Invalid file'));
    const file = new File(['bad'], 'wrong.fbx');
    fireEvent.change(screen.getByLabelText('mate.lab.register'), { target: { files: [file] } });
    await screen.findByText('mate.lab.import_failed');
    expect(m.save).not.toHaveBeenCalled();
    expect(m.load).not.toHaveBeenCalled();
    m.review.mockRejectedValue(new Error('Quota'));
    fireEvent.click(screen.getByRole('button', { name: 'mate.lab.save_review' }));
    await screen.findByText('mate.lab.storage_failed');
    expect(screen.queryByText('mate.lab.saved')).not.toBeInTheDocument();
  });
});
