import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// The property under test is "when may the boot screen replace the app", so
// everything else the shell decides is stubbed out.

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string) => (k === 'boot.lines' ? ['booting'] : k),
  }),
}));
vi.mock('../lib/tauri', () => ({
  isTauri: false,
  checkForUpdates: vi.fn(async () => ({ available: false, latestVersion: '0' })),
}));
const { connection, mounts } = vi.hoisted(() => ({ connection: { connected: false }, mounts: { count: 0 } }));
vi.mock('../contexts/ConnectionContext', () => ({
  useConnection: () => connection,
}));
vi.mock('../services/api', () => ({ api: { getSetupStatus: vi.fn(async () => ({ setup_complete: true })) } }));
vi.mock('../components/SetupWizard', () => ({ SetupWizard: () => null }));
vi.mock('../components/AppLayout', async () => {
  const { useEffect } = await import('react');
  return {
    AppLayout: () => {
      useEffect(() => {
        mounts.count += 1;
      }, []);
      return <div data-testid="app-layout" />;
    },
  };
});
vi.mock('../contexts/AgentContext', () => ({
  AgentProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

import { App } from '../App';

beforeEach(() => {
  connection.connected = false;
  mounts.count = 0;
});

describe('boot screen gate', () => {
  it('covers the app until the kernel answers for the first time', () => {
    const { rerender } = render(<App />);
    expect(screen.getByText('boot.title')).toBeInTheDocument();
    expect(screen.queryByTestId('app-layout')).not.toBeInTheDocument();
    connection.connected = true;
    rerender(<App />);
    expect(screen.queryByText('boot.title')).not.toBeInTheDocument();
    expect(screen.getByTestId('app-layout')).toBeInTheDocument();
  });

  it('keeps the same app mounted when the connection is lost later', () => {
    connection.connected = true;
    const { rerender } = render(<App />);
    expect(mounts.count).toBe(1);
    connection.connected = false;
    rerender(<App />);
    expect(screen.queryByText('boot.title')).not.toBeInTheDocument();
    expect(screen.getByTestId('app-layout')).toBeInTheDocument();
    connection.connected = true;
    rerender(<App />);
    expect(mounts.count).toBe(1);
  });
});
