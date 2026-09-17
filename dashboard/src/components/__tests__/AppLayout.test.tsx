import { act, fireEvent, render, screen } from '@testing-library/react';
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
const navigate = vi.hoisted(() => vi.fn());
vi.mock('react-router-dom', () => ({
  Outlet: () => null,
  useLocation: () => ({ pathname: '/' }),
  useNavigate: () => navigate,
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
  AppSidebar: ({
    onHelpClick,
    onSettingsClick,
    onSearchClick,
  }: {
    onHelpClick?: () => void;
    onSettingsClick?: () => void;
    onSearchClick?: () => void;
  }) => (
    <>
      <button type="button" onClick={onHelpClick} data-testid="sidebar">
        sidebar-help
      </button>
      <button type="button" onClick={onSettingsClick}>
        sidebar-settings
      </button>
      <button type="button" onClick={onSearchClick}>
        sidebar-search
      </button>
    </>
  ),
}));
vi.mock('../CommandPalette', () => ({
  CommandPalette: ({ onClose }: { onClose: () => void }) => (
    <div data-testid="palette">
      <button type="button" onClick={onClose}>
        close-palette
      </button>
    </div>
  ),
}));
vi.mock('../CommandApprovalDeck', () => ({ CommandApprovalDeck: () => null }));
vi.mock('../SecurityGuard', () => ({ SecurityGuard: () => null }));
vi.mock('../HelpContent', () => ({ HelpContent: () => <div>help-content</div> }));
vi.mock('../Modal', () => ({ Modal: ({ children }: { children: ReactNode }) => <div>{children}</div> }));
// Settings is a page now. If the layout ever mounted it again, this would draw.
vi.mock('../SettingsView', () => ({ SettingsView: () => <div data-testid="settings-view" /> }));

import { AppLayout } from '../AppLayout';

beforeEach(() => {
  chrome.hasOverlayTitleBar = false;
  window.localStorage.clear();
  navigate.mockClear();
});

describe('search', () => {
  it('opens and closes on ⌘K and on Ctrl+K, from anywhere', () => {
    render(<AppLayout />);
    expect(screen.queryByTestId('palette')).toBeNull();
    fireEvent.keyDown(window, { key: 'k', metaKey: true });
    expect(screen.getByTestId('palette')).toBeTruthy();
    fireEvent.keyDown(window, { key: 'K', ctrlKey: true });
    expect(screen.queryByTestId('palette')).toBeNull();
  });

  it('does not open on a bare K, which is a letter being typed', () => {
    render(<AppLayout />);
    fireEvent.keyDown(window, { key: 'k' });
    expect(screen.queryByTestId('palette')).toBeNull();
  });

  it("opens from the sidebar's search, and closes when it asks to", () => {
    render(<AppLayout />);
    fireEvent.click(screen.getByText('sidebar-search'));
    expect(screen.getByTestId('palette')).toBeTruthy();
    fireEvent.click(screen.getByText('close-palette'));
    expect(screen.queryByTestId('palette')).toBeNull();
  });
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

describe('the way to settings', () => {
  it('goes to the settings page at the section the update notice asks for', () => {
    render(<AppLayout />);
    act(() => {
      window.dispatchEvent(new CustomEvent('cloto-open-settings', { detail: { section: 'about' } }));
    });
    expect(navigate).toHaveBeenCalledWith('/settings?section=about');
  });

  it('goes to the settings page when the sidebar asks for it', () => {
    render(<AppLayout />);
    fireEvent.click(screen.getByText('sidebar-settings'));
    expect(navigate).toHaveBeenCalledTimes(1);
    expect(navigate).toHaveBeenCalledWith('/settings');
  });

  it('goes to the first section when the notice names none', () => {
    render(<AppLayout />);
    act(() => {
      window.dispatchEvent(new CustomEvent('cloto-open-settings'));
    });
    expect(navigate).toHaveBeenCalledWith('/settings?section=general');
  });

  it('opens no dialog of its own for settings — the page is the destination', () => {
    render(<AppLayout />);
    act(() => {
      window.dispatchEvent(new CustomEvent('cloto-open-settings', { detail: { section: 'about' } }));
    });
    expect(screen.queryByTestId('settings-view')).toBeNull();
  });

  it('still returns to the living room when quick setup finishes', () => {
    render(<AppLayout />);
    act(() => {
      window.dispatchEvent(new CustomEvent('cloto-setup-rerun-complete'));
    });
    expect(navigate).toHaveBeenCalledWith('/');
  });
});
