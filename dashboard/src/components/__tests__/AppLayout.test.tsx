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
    <button type="button" onClick={onHelpClick}>
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
});

describe('the window frame', () => {
  it('draws no bar of its own where the OS draws the title bar', () => {
    const { container } = render(<AppLayout />);
    expect(screen.queryByTestId('window-strip')).toBeNull();
    // Nothing of the old header: no title bar element, no window buttons.
    expect(container.querySelector('header')).toBeNull();
    expect(screen.queryByLabelText('close_window')).toBeNull();
    expect(screen.queryByLabelText('minimize_window')).toBeNull();
  });

  it('leaves a strip to hold the window by where the OS lays its buttons over the page', () => {
    chrome.hasOverlayTitleBar = true;
    render(<AppLayout />);
    const strip = screen.getByTestId('window-strip');
    expect(strip.style.height).toBe('28px');
    // Both halves take hold of the window; neither draws a line under itself.
    const holds = strip.querySelectorAll('[data-tauri-drag-region]');
    expect(holds).toHaveLength(2);
    for (const hold of holds) expect(hold.className).not.toMatch(/border/);
  });

  it('still opens the help, which the sidebar now asks for', () => {
    render(<AppLayout />);
    expect(screen.queryByText('help-content')).toBeNull();
    fireEvent.click(screen.getByText('sidebar-help'));
    expect(screen.getByText('help-content')).toBeTruthy();
  });
});
