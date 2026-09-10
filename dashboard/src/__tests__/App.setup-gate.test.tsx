import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// The property under test is "which shell may render the first-run wizard",
// so everything the wizard branch does NOT decide is stubbed out.

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string) => (k === 'boot.lines' ? ['booting'] : k),
  }),
}));

const { shell } = vi.hoisted(() => ({ shell: { isTauri: false } }));
vi.mock('../lib/tauri', () => ({
  get isTauri() {
    return shell.isTauri;
  },
  checkForUpdates: vi.fn(async () => ({ available: false, latestVersion: '0' })),
}));

const { connection } = vi.hoisted(() => ({ connection: { connected: true } }));
vi.mock('../contexts/ConnectionContext', () => ({
  useConnection: () => connection,
}));

const { getSetupStatus } = vi.hoisted(() => ({ getSetupStatus: vi.fn() }));
vi.mock('../services/api', () => ({ api: { getSetupStatus } }));

vi.mock('../components/SetupWizard', () => ({
  SetupWizard: () => <div data-testid="setup-wizard" />,
}));
vi.mock('../components/AppLayout', () => ({
  AppLayout: () => <div data-testid="app-layout" />,
}));
vi.mock('../contexts/AgentContext', () => ({
  AgentProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

import { App } from '../App';

/** A kernel that is up and reports itself usable — what every deployed one does. */
const CONFIGURED = {
  setup_complete: true,
  setup_in_progress: false,
  mcp_servers_present: false,
  uv_available: false,
  venv_exists: false,
};

beforeEach(() => {
  localStorage.clear();
  vi.clearAllMocks();
  shell.isTauri = false;
  connection.connected = true;
  getSetupStatus.mockResolvedValue(CONFIGURED);
});

describe('first-run wizard gate', () => {
  it('never renders in a browser, whatever this browser has stored', () => {
    render(<App />);
    expect(screen.queryByTestId('setup-wizard')).not.toBeInTheDocument();
    expect(screen.getByTestId('app-layout')).toBeInTheDocument();
  });

  it('stays hidden in a browser even when the kernel reports setup incomplete', () => {
    // Not "the browser trusts the kernel's answer" — the browser is not the
    // machine's first-run shell, so it does not ask. A wizard that replaces the
    // default agent's MCP grants must not be reachable from a remote origin.
    getSetupStatus.mockResolvedValue({ ...CONFIGURED, setup_complete: false });
    render(<App />);
    expect(screen.queryByTestId('setup-wizard')).not.toBeInTheDocument();
  });

  it('renders on a desktop shell that has never completed setup', () => {
    shell.isTauri = true;
    render(<App />);
    expect(screen.getByTestId('setup-wizard')).toBeInTheDocument();
  });

  it('does not render on a desktop shell that has completed setup', () => {
    shell.isTauri = true;
    localStorage.setItem('cloto-setup-completed', '1');
    render(<App />);
    expect(screen.queryByTestId('setup-wizard')).not.toBeInTheDocument();
  });

  it('shows the boot screen, not the wizard, while the kernel is unreachable', () => {
    // "cannot tell" is not "not set up": a disconnected desktop shell with an
    // empty flag must not be walked through 8 steps it may not need.
    shell.isTauri = true;
    connection.connected = false;
    render(<App />);
    expect(screen.queryByTestId('setup-wizard')).not.toBeInTheDocument();
    expect(screen.getByText('boot.title')).toBeInTheDocument();
  });
});
