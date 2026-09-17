import { fireEvent, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const chrome = vi.hoisted(() => ({ hasOverlayTitleBar: false }));
vi.mock('../../lib/tauri', () => ({
  get hasOverlayTitleBar() {
    return chrome.hasOverlayTitleBar;
  },
  OVERLAY_TITLE_BAR_PX: 28,
  isExperimentalBuild: false,
}));
vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (k: string) => k }) }));
vi.mock('react-router-dom', () => ({
  Outlet: () => null,
  useLocation: () => ({ pathname: '/' }),
  useNavigate: () => vi.fn(),
  useNavigationType: () => 'POP',
}));
const agentCtx = vi.hoisted(() => ({ agents: [], setSelectedAgentId: () => {} }));
vi.mock('../../contexts/AgentContext', () => ({ useAgentContext: () => agentCtx }));
vi.mock('../../contexts/ActionsContext', () => ({
  ActionsProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
}));
vi.mock('../../contexts/ConversationContext', () => ({
  ConversationProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
}));
vi.mock('../../pages/AgentPage', () => ({ AgentPage: () => <div>agent-page</div> }));
vi.mock('../AppSidebar', () => ({
  AppSidebar: ({ onHelpClick }: { onHelpClick?: () => void }) => (
    <button type="button" onClick={onHelpClick} data-testid="sidebar">
      sidebar-help
    </button>
  ),
}));
vi.mock('../CommandApprovalDeck', () => ({ CommandApprovalDeck: () => null }));
vi.mock('../SecurityGuard', () => ({ SecurityGuard: () => null }));
vi.mock('../HelpContent', () => ({ HelpContent: () => <div>help-content</div> }));
vi.mock('../Modal', () => ({ Modal: ({ children }: { children: ReactNode }) => <div>{children}</div> }));

import { AppLayout } from '../AppLayout';

beforeEach(() => {
  chrome.hasOverlayTitleBar = false;
  window.localStorage.clear();
});

describe('the window frame', () => {
  it('draws no window buttons of its own: the bar carries the sidebar toggle, back and forward, and nothing else', () => {
    const { container } = render(<AppLayout />);
    const bar = screen.getByTestId('window-bar');
    expect([...bar.querySelectorAll('button')].map((b) => b.getAttribute('aria-label'))).toEqual([
      'hide_sidebar',
      'go_back',
      'go_forward',
    ]);
    expect(container.querySelector('header')).toBeNull();
    expect(screen.queryByLabelText('close_window')).toBeNull();
    expect(screen.queryByLabelText('minimize_window')).toBeNull();
    expect(bar.className).not.toMatch(/border/);
  });

  it('is what the window is held by, everywhere a button is not', () => {
    chrome.hasOverlayTitleBar = true;
    render(<AppLayout />);
    const bar = screen.getByTestId('window-bar');
    expect(bar.className).toContain('overlay');
    const holds = bar.querySelectorAll('[data-tauri-drag-region]');
    expect(holds).toHaveLength(2);
    // A button that dragged the window would never be clicked.
    for (const b of bar.querySelectorAll('button')) expect(b.hasAttribute('data-tauri-drag-region')).toBe(false);
  });

  it('hides and shows the sidebar, and remembers which', () => {
    const first = render(<AppLayout />);
    expect(screen.getByTestId('sidebar')).toBeTruthy();
    fireEvent.click(screen.getByLabelText('hide_sidebar'));
    expect(screen.queryByTestId('sidebar')).toBeNull();
    // The bar stops continuing a surface that is no longer under it.
    expect(screen.getByTestId('window-bar').querySelector('.over-side')).toBeNull();
    first.unmount();

    render(<AppLayout />);
    expect(screen.queryByTestId('sidebar')).toBeNull();
    fireEvent.click(screen.getByLabelText('show_sidebar'));
    expect(screen.getByTestId('sidebar')).toBeTruthy();
    expect(screen.getByTestId('window-bar').querySelector('.over-side')).not.toBeNull();
  });

  it('still opens the help, which the sidebar now asks for', () => {
    render(<AppLayout />);
    expect(screen.queryByText('help-content')).toBeNull();
    fireEvent.click(screen.getByText('sidebar-help'));
    expect(screen.getByText('help-content')).toBeTruthy();
  });
});
