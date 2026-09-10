import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}));

const { shell } = vi.hoisted(() => ({ shell: { isTauri: false } }));
vi.mock('../../../lib/tauri', () => ({
  get isTauri() {
    return shell.isTauri;
  },
  applyUpdate: vi.fn(),
  checkForUpdates: vi.fn(),
  UPDATE_CHANNEL_STORAGE_KEY: 'cloto-update-channel',
  UPDATE_CHANNELS: ['stable'],
}));

vi.mock('../../SetupWizard', () => ({
  SetupWizard: () => <div data-testid="setup-wizard" />,
}));

import { AboutSection } from '../AboutSection';

beforeEach(() => {
  localStorage.clear();
  shell.isTauri = false;
});

describe('re-run setup affordance', () => {
  it('is absent in a browser, so the wizard has no entry point there', () => {
    render(<AboutSection />);
    expect(screen.queryByText('about.rerun_setup')).not.toBeInTheDocument();
    expect(screen.queryByTestId('setup-wizard')).not.toBeInTheDocument();
  });

  it('is offered on the desktop shell, where the wizard can complete', () => {
    shell.isTauri = true;
    render(<AboutSection />);
    expect(screen.getByText('about.rerun_setup')).toBeInTheDocument();
  });
});
